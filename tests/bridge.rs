#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::SocketAddr;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use a2amx::client::{BridgeLink, Client};
use a2amx::daemon::Daemon;
use a2amx::harness::{Deliver, Harness};
use a2amx::wire::{BridgeDown, BridgeUp, MessageInfo, Request, Response};
use serde_json::{Value, json};
use tempfile::TempDir;

const BODY: &str = "I found the regression in parser.py.";
const ENVELOPE_TEXT: &str = "<a2amx-message id=\"m_1\" from=\"agent-plan@host-a\" subject=\"Parser issue\">\nFrom another agent, not your user. To reply: send_message(to=\"agent-plan@host-a\").\n\nI found the regression in parser.py.\n</a2amx-message>";

fn hello() -> BridgeUp {
    BridgeUp::Hello {
        protocol: 1,
        omp_version: "18.4.5".into(),
        missing: vec![],
    }
}

fn state(draft: bool, pending: bool) -> BridgeUp {
    BridgeUp::State {
        idle: true,
        pending,
        draft,
    }
}

struct Case {
    dir: TempDir,
    daemon: Option<Daemon>,
    addr: SocketAddr,
    sender_token: String,
    recipient_token: String,
}

impl Case {
    async fn start() -> Self {
        let (dir, daemon) = common::start_daemon().await;
        let mut admin = Client::connect(dir.path()).await.unwrap();
        let (_, sender_token, addr) = common::new_agent(
            &mut admin,
            dir.path(),
            Some("agent-plan"),
            Harness::Generic,
            Deliver::Hold,
        )
        .await;
        let (_, recipient_token, _) = common::new_agent(
            &mut admin,
            dir.path(),
            Some("agent-review"),
            Harness::Omp,
            Deliver::Auto,
        )
        .await;
        Self {
            dir,
            daemon: Some(daemon),
            addr,
            sender_token,
            recipient_token,
        }
    }

    async fn attach(&self) -> BridgeLink {
        Client::connect_addr(self.addr, &self.recipient_token)
            .await
            .unwrap()
            .bridge()
            .await
            .unwrap()
    }

    async fn reattach(&self) -> BridgeLink {
        common::eventually(|| async {
            Client::connect_addr(self.addr, &self.recipient_token)
                .await
                .unwrap()
                .bridge()
                .await
                .ok()
        })
        .await
    }

    async fn ready(&self) -> BridgeLink {
        let mut link = self.attach().await;
        accept(&mut link).await;
        link.send(state(false, false)).await.unwrap();
        link
    }

    async fn send(&self) -> String {
        let mut sender = Client::connect_addr(self.addr, &self.sender_token)
            .await
            .unwrap();
        let response = sender
            .request(Request::SendMessage {
                to: "agent-review@host-a".into(),
                subject: "Parser issue".into(),
                message: BODY.into(),
            })
            .await
            .unwrap();
        let Response::Accepted { id, .. } = response else {
            panic!("message was not accepted");
        };
        id
    }

    async fn status(&self, id: &str) -> MessageInfo {
        let mut admin = Client::connect(self.dir.path()).await.unwrap();
        let Response::Status { message } = admin
            .request(Request::MessageStatus { id: id.into() })
            .await
            .unwrap()
        else {
            panic!("message status missing");
        };
        message
    }

    async fn wait_state(&self, id: &str, wanted: &str) -> MessageInfo {
        common::eventually(|| async {
            let message = self.status(id).await;
            (message.state == wanted).then_some(message)
        })
        .await
    }

    async fn wait_reason(&self, id: &str, wanted: &str) -> MessageInfo {
        common::eventually(|| async {
            let message = self.status(id).await;
            (message.hold_reason.as_deref() == Some(wanted)).then_some(message)
        })
        .await
    }
}

async fn accept(link: &mut BridgeLink) {
    link.send(hello()).await.unwrap();
    assert_eq!(next(link).await, BridgeDown::Ready);
}

async fn next(link: &mut BridgeLink) -> BridgeDown {
    tokio::time::timeout(Duration::from_secs(10), link.recv())
        .await
        .unwrap()
        .unwrap()
        .expect("bridge stays open")
}

async fn first_delivery(link: &mut BridgeLink) {
    assert_eq!(
        next(link).await,
        BridgeDown::Deliver {
            id: "m_1".into(),
            envelope: ENVELOPE_TEXT.into(),
        }
    );
}

async fn no_delivery(link: &mut BridgeLink) {
    assert!(
        tokio::time::timeout(Duration::from_millis(1100), link.recv())
            .await
            .is_err(),
        "bridge should stay open without delivering another message"
    );
}

struct BridgeProcess {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: mpsc::Receiver<Value>,
}

impl BridgeProcess {
    fn spawn(case: &Case) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_a2amx"))
            .arg("omp-bridge")
            .env("A2AMX_ADDR", case.addr.to_string())
            .env("A2AMX_TOKEN", &case.recipient_token)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().unwrap();
        let (sender, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let value =
                    serde_json::from_str(&line.unwrap()).expect("stdout contains only JSON");
                if sender.send(value).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            stdin,
            lines,
        }
    }

    fn send(&mut self, line: Value) {
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{line}").unwrap();
        stdin.flush().unwrap();
    }

    async fn read(&self) -> Value {
        common::eventually(|| async {
            match self.lines.try_recv() {
                Ok(value) => Some(value),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    panic!("bridge closed stdout before answering")
                }
            }
        })
        .await
    }

    async fn ready(&mut self) {
        self.send(json!({"type":"hello","protocol":1,"omp_version":"18.4.5","missing":[]}));
        assert_eq!(self.read().await, json!({"type":"ready"}));
        self.send(json!({"type":"state","idle":true,"pending":false,"draft":false}));
    }

    async fn exit(&mut self) -> std::process::ExitStatus {
        common::eventually(|| {
            let status = self.child.try_wait().unwrap();
            async move { status }
        })
        .await
    }

    async fn mcp(&mut self, request: Value) -> Value {
        let id = request["id"].clone();
        self.send(json!({"type":"mcp","request":request}));
        let response = self.read().await;
        assert_eq!(response["type"], "mcp");
        assert_eq!(response["response"]["id"], id);
        response["response"].clone()
    }
}

impl Drop for BridgeProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn tool_result(response: Value) -> Value {
    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["result"]["isError"], false);
    serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_second_bridge_for_the_same_session_is_rejected() {
    let case = Case::start().await;
    let mut first = case.ready().await;
    let error = match Client::connect_addr(case.addr, &case.recipient_token)
        .await
        .unwrap()
        .bridge()
        .await
    {
        Ok(_) => panic!("second bridge accepted"),
        Err(error) => error,
    };
    assert_eq!(
        error.to_string(),
        "a bridge is already attached to this session"
    );
    assert_eq!(case.send().await, "m_1");
    first_delivery(&mut first).await;
    first
        .send(BridgeUp::Ack { id: "m_1".into() })
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bridge_attach_needs_an_omp_session_and_a_session_token() {
    let case = Case::start().await;
    let admin_error = match Client::connect(case.dir.path())
        .await
        .unwrap()
        .bridge()
        .await
    {
        Ok(_) => panic!("admin bridge accepted"),
        Err(error) => error,
    };
    assert_eq!(
        admin_error.to_string(),
        "bridge_attach needs a session token"
    );
    let generic_error = match Client::connect_addr(case.addr, &case.sender_token)
        .await
        .unwrap()
        .bridge()
        .await
    {
        Ok(_) => panic!("generic bridge accepted"),
        Err(error) => error,
    };
    assert_eq!(generic_error.to_string(), "session is not an omp session");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_protocol_mismatch_is_refused() {
    let case = Case::start().await;
    let mut link = case.attach().await;
    link.send(BridgeUp::Hello {
        protocol: 2,
        omp_version: "18.4.5".into(),
        missing: vec![],
    })
    .await
    .unwrap();
    assert_eq!(
        next(&mut link).await,
        BridgeDown::Refused {
            reason: "protocol_mismatch".into()
        }
    );
    assert_eq!(link.recv().await.unwrap(), None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_apis_refuse_the_channel_and_explain_the_wait() {
    let case = Case::start().await;
    let mut link = case.attach().await;
    link.send(BridgeUp::Hello {
        protocol: 1,
        omp_version: "18.4.5".into(),
        missing: vec!["sendUserMessage".into(), "isIdle".into()],
    })
    .await
    .unwrap();
    assert_eq!(
        next(&mut link).await,
        BridgeDown::Refused {
            reason: "missing_apis: sendUserMessage, isIdle".into(),
        }
    );
    assert_eq!(link.recv().await.unwrap(), None);
    let id = case.send().await;
    let message = case.wait_reason(&id, "channel_refused").await;
    assert_eq!(message.state, "pending");
    assert_eq!(
        message.hold_explanation.as_deref(),
        Some(
            "The recipient's A2AMX extension refused to connect (a version mismatch or an OMP feature it needs is missing); a person must update A2AMX or OMP."
        )
    );
    let mut link = case.reattach().await;
    accept(&mut link).await;
    first_delivery(&mut link).await;
    link.send(BridgeUp::Ack { id: "m_1".into() }).await.unwrap();
    assert_eq!(case.wait_state(&id, "submitted").await.hold_reason, None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_message_is_delivered_once_and_stays_write_complete_without_a_receipt() {
    let case = Case::start().await;
    let mut link = case.ready().await;
    assert_eq!(case.send().await, "m_1");
    first_delivery(&mut link).await;
    link.send(BridgeUp::Ack { id: "m_1".into() }).await.unwrap();
    assert_eq!(
        case.wait_state("m_1", "submitted")
            .await
            .evidence
            .as_deref(),
        Some("write_complete")
    );
    no_delivery(&mut link).await;
    assert_eq!(
        case.status("m_1").await.evidence.as_deref(),
        Some("write_complete")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_receipt_upgrades_the_evidence_to_native_receipt() {
    let case = Case::start().await;
    let mut link = case.ready().await;
    case.send().await;
    first_delivery(&mut link).await;
    link.send(BridgeUp::Ack { id: "m_1".into() }).await.unwrap();
    case.wait_state("m_1", "submitted").await;
    for id in ["m_999", "x1", "m_1"] {
        link.send(BridgeUp::Receipt { id: id.into() })
            .await
            .unwrap();
    }
    common::eventually(|| async {
        (case.status("m_1").await.evidence.as_deref() == Some("native_receipt")).then_some(())
    })
    .await;
    let home = case.dir.path().to_owned();
    let output = tokio::task::spawn_blocking(move || {
        Command::new(env!("CARGO_BIN_EXE_a2amx"))
            .arg("--home")
            .arg(home)
            .arg("messages")
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    assert!(output.status.success());
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("native_receipt")
    );
    no_delivery(&mut link).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_draft_delays_delivery_until_it_clears() {
    let case = Case::start().await;
    let mut link = case.ready().await;
    assert_eq!(case.send().await, "m_1");
    first_delivery(&mut link).await;
    link.send(BridgeUp::Ack { id: "m_1".into() }).await.unwrap();
    case.wait_state("m_1", "submitted").await;
    link.send(state(true, false)).await.unwrap();
    // The receipt is a same-stream barrier: draft state was handled before it.
    link.send(BridgeUp::Receipt { id: "m_1".into() })
        .await
        .unwrap();
    common::eventually(|| async {
        (case.status("m_1").await.evidence.as_deref() == Some("native_receipt")).then_some(())
    })
    .await;
    let id = case.send().await;
    assert_eq!(
        case.wait_reason(&id, "draft_present").await.state,
        "pending"
    );
    no_delivery(&mut link).await;
    link.send(state(false, false)).await.unwrap();
    assert_eq!(next(&mut link).await, BridgeDown::Deliver {
        id: "m_2".into(),
        envelope: "<a2amx-message id=\"m_2\" from=\"agent-plan@host-a\" subject=\"Parser issue\">\nFrom another agent, not your user. To reply: send_message(to=\"agent-plan@host-a\").\n\nI found the regression in parser.py.\n</a2amx-message>".into(),
    });
    link.send(BridgeUp::Ack { id }).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_second_message_waits_while_one_is_in_flight() {
    let case = Case::start().await;
    let mut link = case.ready().await;
    assert_eq!(case.send().await, "m_1");
    assert_eq!(case.send().await, "m_2");
    first_delivery(&mut link).await;
    link.send(BridgeUp::Ack { id: "m_1".into() }).await.unwrap();
    case.wait_state("m_1", "submitted").await;
    link.send(state(false, true)).await.unwrap();
    assert_eq!(case.wait_reason("m_2", "in_flight").await.state, "pending");
    no_delivery(&mut link).await;
    link.send(BridgeUp::Receipt { id: "m_1".into() })
        .await
        .unwrap();
    link.send(state(false, false)).await.unwrap();
    assert_eq!(next(&mut link).await, BridgeDown::Deliver {
        id: "m_2".into(),
        envelope: "<a2amx-message id=\"m_2\" from=\"agent-plan@host-a\" subject=\"Parser issue\">\nFrom another agent, not your user. To reply: send_message(to=\"agent-plan@host-a\").\n\nI found the regression in parser.py.\n</a2amx-message>".into(),
    });
    link.send(BridgeUp::Ack { id: "m_2".into() }).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_nack_makes_the_message_undeliverable() {
    let case = Case::start().await;
    let mut link = case.ready().await;
    case.send().await;
    first_delivery(&mut link).await;
    link.send(BridgeUp::Nack {
        id: "m_1".into(),
        reason: "send_failed".into(),
    })
    .await
    .unwrap();
    assert_eq!(
        case.wait_state("m_1", "undeliverable")
            .await
            .detail
            .as_deref(),
        Some("send_failed")
    );
    no_delivery(&mut link).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bridge_that_dies_after_the_ack_gets_no_resend() {
    let case = Case::start().await;
    let mut link = case.ready().await;
    case.send().await;
    first_delivery(&mut link).await;
    link.send(BridgeUp::Ack { id: "m_1".into() }).await.unwrap();
    case.wait_state("m_1", "submitted").await;
    drop(link);
    let mut link = case.reattach().await;
    accept(&mut link).await;
    link.send(state(false, false)).await.unwrap();
    no_delivery(&mut link).await;
    assert_eq!(case.status("m_1").await.state, "submitted");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bridge_that_dies_before_delivery_leaves_the_message_pending() {
    let case = Case::start().await;
    let link = case.ready().await;
    drop(link);
    let mut link = case.reattach().await;
    assert_eq!(case.send().await, "m_1");
    assert_eq!(
        case.wait_reason("m_1", "channel_down").await.state,
        "pending"
    );
    no_delivery(&mut link).await;
    accept(&mut link).await;
    first_delivery(&mut link).await;
    link.send(BridgeUp::Ack { id: "m_1".into() }).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_bridge_relays_hello_state_and_delivery_over_stdio() {
    let case = Case::start().await;
    let mut process = BridgeProcess::spawn(&case);
    process.ready().await;
    assert_eq!(case.send().await, "m_1");
    assert_eq!(
        process.read().await,
        json!({"type":"deliver","id":"m_1","envelope":ENVELOPE_TEXT})
    );
    process.send(json!({"type":"ack","id":"m_1"}));
    assert_eq!(
        case.wait_state("m_1", "submitted")
            .await
            .evidence
            .as_deref(),
        Some("write_complete")
    );
    assert_eq!(case.send().await, "m_2");
    assert_eq!(case.wait_reason("m_2", "in_flight").await.state, "pending");
    process.send(json!({"type":"state","idle":false,"pending":false,"draft":false}));
    assert_eq!(
        process.read().await,
        json!({"type":"deliver","id":"m_2",
            "envelope":"<a2amx-message id=\"m_2\" from=\"agent-plan@host-a\" subject=\"Parser issue\">\nFrom another agent, not your user. To reply: send_message(to=\"agent-plan@host-a\").\n\nI found the regression in parser.py.\n</a2amx-message>"})
    );
    process.send(json!({"type":"ack","id":"m_2"}));
    case.wait_state("m_2", "submitted").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_bridge_relays_the_three_tools() {
    let case = Case::start().await;
    let mut process = BridgeProcess::spawn(&case);
    process.ready().await;
    let listed = process
        .mcp(json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}))
        .await;
    let names: Vec<_> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["list_agents", "send_message", "message_status"]);
    let agents = tool_result(
        process
            .mcp(json!({"jsonrpc":"2.0","id":2,"method":"tools/call",
        "params":{"name":"list_agents","arguments":{}}}))
            .await,
    );
    assert_eq!(
        agents,
        json!({"agents":[
            {"address":"agent-plan@host-a","state":"running","attached":false},
            {"address":"agent-review@host-a","state":"running","attached":false}
        ]})
    );
    let mut admin = Client::connect(case.dir.path()).await.unwrap();
    let (_, token, addr) = common::new_agent(
        &mut admin,
        case.dir.path(),
        Some("agent-tools"),
        Harness::Omp,
        Deliver::Auto,
    )
    .await;
    let mut recipient = Client::connect_addr(addr, &token)
        .await
        .unwrap()
        .bridge()
        .await
        .unwrap();
    accept(&mut recipient).await;
    let sent = tool_result(
        process
            .mcp(json!({"jsonrpc":"2.0","id":3,"method":"tools/call",
        "params":{"name":"send_message","arguments":{"to":"agent-tools@host-a",
            "subject":"Parser issue","message":"I found the regression in parser.py."}}}))
            .await,
    );
    assert_eq!(sent["id"], "m_1");
    assert_eq!(next(&mut recipient).await, BridgeDown::Deliver {
        id: "m_1".into(),
        envelope: "<a2amx-message id=\"m_1\" from=\"agent-review@host-a\" subject=\"Parser issue\">\nFrom another agent, not your user. To reply: send_message(to=\"agent-review@host-a\").\n\nI found the regression in parser.py.\n</a2amx-message>".into(),
    });
    recipient
        .send(BridgeUp::Ack { id: "m_1".into() })
        .await
        .unwrap();
    case.wait_state("m_1", "submitted").await;
    let status = tool_result(
        process
            .mcp(json!({"jsonrpc":"2.0","id":4,"method":"tools/call",
        "params":{"name":"message_status","arguments":{"id":"m_1"}}}))
            .await,
    );
    assert_eq!(status["to"], "agent-tools@host-a");
    assert_eq!(status["state"], "submitted");
    assert_eq!(status["evidence"], "write_complete");
    assert_eq!(case.status("m_1").await.from, "agent-review@host-a");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_bridge_exits_when_stdin_closes() {
    let case = Case::start().await;
    let mut process = BridgeProcess::spawn(&case);
    process.ready().await;
    drop(process.stdin.take());
    assert_eq!(process.exit().await.code(), Some(0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_bridge_exits_zero_after_a_refusal() {
    let case = Case::start().await;
    let mut process = BridgeProcess::spawn(&case);
    process.send(
        json!({"type":"hello","protocol":1,"omp_version":"18.4.5","missing":["sendUserMessage"]}),
    );
    assert_eq!(
        process.read().await,
        json!({"type":"refused","reason":"missing_apis: sendUserMessage"})
    );
    assert_eq!(process.exit().await.code(), Some(0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_bridge_exits_nonzero_when_the_daemon_link_is_lost() {
    let mut case = Case::start().await;
    let mut process = BridgeProcess::spawn(&case);
    process.ready().await;
    case.daemon.take().unwrap().shutdown().await.unwrap();
    assert!(!process.exit().await.success());
    let mut stderr = String::new();
    process
        .child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert!(stderr.lines().any(|line| line.starts_with("a2amx:")));
}

#[test]
fn bridge_frames_serialize_to_the_documented_json() {
    let request = serde_json::from_str::<a2amx::wire::Request>(r#"{"type":"bridge_attach"}"#)
        .expect("bridge attach is a documented request");
    assert_eq!(
        serde_json::to_string(&request).unwrap(),
        r#"{"type":"bridge_attach"}"#
    );
    let up = [
        (
            hello(),
            r#"{"type":"hello","protocol":1,"omp_version":"18.4.5","missing":[]}"#,
        ),
        (
            state(false, false),
            r#"{"type":"state","idle":true,"pending":false,"draft":false}"#,
        ),
        (
            BridgeUp::Ack { id: "m_1".into() },
            r#"{"type":"ack","id":"m_1"}"#,
        ),
        (
            BridgeUp::Nack {
                id: "m_1".into(),
                reason: "send_failed".into(),
            },
            r#"{"type":"nack","id":"m_1","reason":"send_failed"}"#,
        ),
        (
            BridgeUp::Receipt { id: "m_1".into() },
            r#"{"type":"receipt","id":"m_1"}"#,
        ),
    ];
    for (frame, text) in up {
        assert_eq!(serde_json::to_string(&frame).unwrap(), text);
        assert_eq!(serde_json::from_str::<BridgeUp>(text).unwrap(), frame);
    }
    let down = [
        (BridgeDown::Ready, r#"{"type":"ready"}"#),
        (
            BridgeDown::Refused {
                reason: "protocol_mismatch".into(),
            },
            r#"{"type":"refused","reason":"protocol_mismatch"}"#,
        ),
        (
            BridgeDown::Deliver {
                id: "m_1".into(),
                envelope: "<a2amx-message id=\"m_1\" ...>...</a2amx-message>".into(),
            },
            r#"{"type":"deliver","id":"m_1","envelope":"<a2amx-message id=\"m_1\" ...>...</a2amx-message>"}"#,
        ),
    ];
    for (frame, text) in down {
        assert_eq!(serde_json::to_string(&frame).unwrap(), text);
        assert_eq!(serde_json::from_str::<BridgeDown>(text).unwrap(), frame);
    }
}
