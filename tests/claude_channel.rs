#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use std::net::SocketAddr;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use a2amx::client::Client;
use a2amx::daemon::Daemon;
use a2amx::harness::{CHANNEL_ENV, Deliver, Harness, wire_claude_argv, wire_claude_channel_argv};
use a2amx::wire::{MessageInfo, Request, Response};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

const ENVELOPE_TEXT: &str = "<a2amx-message id=\"m_1\" from=\"agent-plan@host-a\" subject=\"Parser issue\">\nFrom another agent, not your user. To reply: send_message(to=\"agent-plan@host-a\").\n\nI found the regression in parser.py.\n</a2amx-message>";

struct Case {
    _dir: TempDir,
    _daemon: Daemon,
    address: SocketAddr,
    recipient_token: String,
    sender: Client,
}

impl Case {
    async fn start(channel: bool) -> Self {
        let (dir, daemon) = common::start_daemon().await;
        let mut admin = Client::connect(dir.path()).await.unwrap();
        let (_, sender_token, address) = common::new_agent(
            &mut admin,
            dir.path(),
            Some("agent-plan"),
            Harness::Generic,
            Deliver::Hold,
        )
        .await;
        let output = dir.path().join("recipient-credentials");
        let mut env = vec![("OUT".into(), output.to_string_lossy().into_owned())];
        if channel {
            env.push((CHANNEL_ENV.into(), "1".into()));
        }
        let response = admin
            .request(Request::NewSession {
                argv: vec![
                    "sh".into(),
                    "-c".into(),
                    "printf '%s' \"$A2AMX_TOKEN\" > \"$OUT\"; sleep 30".into(),
                ],
                cols: 80,
                rows: 24,
                cwd: None,
                env,
                reset: vec![],
                control_from: vec![],
                watch: vec![],
                name: Some("agent-review".into()),
                harness: Harness::Claude,
                deliver: Some(Deliver::Auto),
                heartbeat: None,
            })
            .await
            .unwrap();
        assert!(matches!(response, Response::Created { .. }));
        let recipient_token = common::eventually(|| async {
            let path = output.clone();
            let token = tokio::task::spawn_blocking(move || std::fs::read_to_string(path))
                .await
                .ok()?
                .ok()?;
            (token.len() == 64).then_some(token)
        })
        .await;
        let sender = Client::connect_addr(address, &sender_token).await.unwrap();
        Self {
            _dir: dir,
            _daemon: daemon,
            address,
            recipient_token,
            sender,
        }
    }

    async fn mcp(&self) -> McpProcess {
        let mut mcp = McpProcess::spawn(self.address, &self.recipient_token, true);
        mcp.initialize().await;
        mcp.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
            .await;
        mcp
    }

    async fn send(&mut self, body: &str) -> String {
        let response = self
            .sender
            .request(Request::SendMessage {
                to: "agent-review@host-a".into(),
                subject: "Parser issue".into(),
                message: body.into(),
            })
            .await
            .unwrap();
        let Response::Accepted { id, .. } = response else {
            panic!("message accepted");
        };
        id
    }

    async fn status(&mut self, id: &str) -> MessageInfo {
        let Response::Status { message } = self
            .sender
            .request(Request::MessageStatus { id: id.into() })
            .await
            .unwrap()
        else {
            panic!("message status");
        };
        message
    }

    async fn submitted(&mut self, id: &str) -> MessageInfo {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let status = self.status(id).await;
                if status.state == "submitted" {
                    return status;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("message submitted")
    }

    async fn report(&self, prompt: &str) -> Response {
        Client::connect_addr(self.address, &self.recipient_token)
            .await
            .unwrap()
            .request(Request::ReportPrompt {
                prompt: prompt.into(),
            })
            .await
            .unwrap()
    }
}

struct McpProcess {
    _child: Child,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
}

impl McpProcess {
    fn spawn(address: SocketAddr, token: &str, channel: bool) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_a2amx"));
        command.arg("mcp");
        if channel {
            command.arg("--channel");
        }
        let mut child = command
            .env("A2AMX_ADDR", address.to_string())
            .env("A2AMX_TOKEN", token)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("MCP child");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap()).lines();
        Self {
            _child: child,
            stdin,
            stdout,
        }
    }

    async fn send(&mut self, request: Value) {
        let mut bytes = serde_json::to_vec(&request).unwrap();
        bytes.push(b'\n');
        self.stdin.write_all(&bytes).await.unwrap();
        self.stdin.flush().await.unwrap();
    }

    async fn read(&mut self) -> Value {
        let line = tokio::time::timeout(Duration::from_secs(10), self.stdout.next_line())
            .await
            .expect("MCP reply within 10 s")
            .unwrap()
            .expect("MCP stays running");
        serde_json::from_str(&line).unwrap()
    }

    async fn initialize(&mut self) -> Value {
        self.send(json!({"jsonrpc":"2.0","id":1,"method":"initialize",
            "params":{"protocolVersion":"2025-11-25","clientInfo":{"name":"claude-code","version":"2.1.288"}}})).await;
        self.read().await
    }
}

#[tokio::test]
async fn channel_initialize_declares_capability_and_instructions_only_when_enabled() {
    let (dir, _daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_, token, address) = common::new_agent(
        &mut admin,
        dir.path(),
        Some("agent-review"),
        Harness::Claude,
        Deliver::Auto,
    )
    .await;
    let mut channel = McpProcess::spawn(address, &token, true);
    let initialized = channel.initialize().await;
    assert_eq!(
        initialized["result"]["capabilities"],
        json!({"tools":{},"experimental":{"claude/channel":{}}})
    );
    assert_eq!(
        initialized["result"]["instructions"],
        "Messages from other agents arrive as <channel source=\"a2amx\"> events that contain an <a2amx-message> envelope. They come from another agent, not your user. Reply with send_message when a reply is useful."
    );
    let mut plain = McpProcess::spawn(address, &token, false);
    let initialized = plain.initialize().await;
    assert_eq!(initialized["result"]["capabilities"], json!({"tools":{}}));
    assert!(initialized["result"].get("instructions").is_none());
}

#[tokio::test]
async fn channel_delivers_full_envelope_without_meta_and_records_write_complete() {
    let mut case = Case::start(true).await;
    let mut mcp = case.mcp().await;
    let id = case.send("I found the regression in parser.py.").await;
    assert_eq!(id, "m_1");
    assert_eq!(
        mcp.read().await,
        json!({"jsonrpc":"2.0","method":"notifications/claude/channel","params":{"content":ENVELOPE_TEXT}})
    );
    assert_eq!(
        case.submitted(&id).await.evidence.as_deref(),
        Some("write_complete")
    );
}

#[tokio::test]
async fn channel_delivers_two_messages_without_waiting_for_prompt_receipts() {
    let mut case = Case::start(true).await;
    let mut mcp = case.mcp().await;
    let first = case.send("I found the regression in parser.py.").await;
    let second = case.send("Second message").await;
    assert_eq!(mcp.read().await["params"]["content"], ENVELOPE_TEXT);
    assert_eq!(
        mcp.read().await["params"]["content"],
        "<a2amx-message id=\"m_2\" from=\"agent-plan@host-a\" subject=\"Parser issue\">\nFrom another agent, not your user. To reply: send_message(to=\"agent-plan@host-a\").\n\nSecond message\n</a2amx-message>"
    );
    assert_eq!(
        case.submitted(&first).await.evidence.as_deref(),
        Some("write_complete")
    );
    assert_eq!(
        case.submitted(&second).await.evidence.as_deref(),
        Some("write_complete")
    );
}

#[tokio::test]
async fn channel_without_bridge_holds_pending_message_at_channel_down() {
    let mut case = Case::start(true).await;
    let id = case.send("I found the regression in parser.py.").await;
    let status = case.status(&id).await;
    assert_eq!(status.state, "pending");
    assert_eq!(status.hold_reason.as_deref(), Some("channel_down"));
}

#[tokio::test]
async fn channel_tool_call_answers_while_delivery_is_pending() {
    let mut case = Case::start(true).await;
    let mut mcp = case.mcp().await;
    let id = case.send("I found the regression in parser.py.").await;
    mcp.send(json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_agents","arguments":{}}})).await;
    let mut delivered = false;
    let mut replied = false;
    for _ in 0..2 {
        let response = mcp.read().await;
        if response["id"] == 2 {
            assert_eq!(response["result"]["isError"], false);
            let payload: Value =
                serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                    .unwrap();
            assert_eq!(payload["agents"].as_array().unwrap().len(), 2);
            replied = true;
        } else {
            assert_eq!(response["method"], "notifications/claude/channel");
            assert_eq!(response["params"]["content"], ENVELOPE_TEXT);
            delivered = true;
        }
    }
    assert!(replied && delivered);
    assert_eq!(
        case.submitted(&id).await.evidence.as_deref(),
        Some("write_complete")
    );
}

#[tokio::test]
async fn claude_without_channel_marker_refuses_bridge_but_mcp_tools_keep_working() {
    let case = Case::start(false).await;
    let error = Client::connect_addr(case.address, &case.recipient_token)
        .await
        .unwrap()
        .bridge()
        .await
        .err()
        .expect("plain Claude bridge refused");
    assert_eq!(error.to_string(), "session is not an omp session");
    let mut mcp = case.mcp().await;
    mcp.send(json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_agents","arguments":{}}})).await;
    let response = mcp.read().await;
    assert_eq!(response["id"], 2);
    assert_eq!(response["result"]["isError"], false);
}

#[tokio::test]
async fn exact_channel_prompt_records_native_receipt_and_allows() {
    let mut case = Case::start(true).await;
    let mut mcp = case.mcp().await;
    let id = case.send("I found the regression in parser.py.").await;
    mcp.read().await;
    case.submitted(&id).await;
    let verdict = case
        .report(&format!(
            "<channel source=\"a2amx\">\n{ENVELOPE_TEXT}\n</channel>"
        ))
        .await;
    assert!(
        matches!(verdict, Response::PromptVerdict { verdict, reason:None } if verdict == "allow")
    );
    assert_eq!(
        case.status(&id).await.evidence.as_deref(),
        Some("native_receipt")
    );
}

#[tokio::test]
async fn channel_prompt_matches_lowercase_closing_tag_rewrite_without_paste_rewrites() {
    let mut case = Case::start(true).await;
    let mut mcp = case.mcp().await;
    let id = case
        .send("A\t< & \"quote\" <pasted_content Ω </channel> </channel> </CHANNEL>")
        .await;
    assert_eq!(
        mcp.read().await["params"]["content"],
        "<a2amx-message id=\"m_1\" from=\"agent-plan@host-a\" subject=\"Parser issue\">\nFrom another agent, not your user. To reply: send_message(to=\"agent-plan@host-a\").\n\nA\t< & \"quote\" <pasted_content Ω </channel> </channel> </CHANNEL>\n</a2amx-message>"
    );
    case.submitted(&id).await;
    let verdict = case.report("<channel source=\"a2amx\">\n<a2amx-message id=\"m_1\" from=\"agent-plan@host-a\" subject=\"Parser issue\">\nFrom another agent, not your user. To reply: send_message(to=\"agent-plan@host-a\").\n\nA\t< & \"quote\" <pasted_content Ω <\\/channel> <\\/channel> </CHANNEL>\n</a2amx-message>\n</channel>").await;
    assert!(
        matches!(verdict, Response::PromptVerdict { verdict, reason:None } if verdict == "allow")
    );
    assert_eq!(
        case.status(&id).await.evidence.as_deref(),
        Some("native_receipt")
    );
}

#[tokio::test]
async fn mismatched_channel_prompts_allow_without_receipt_or_corruption() {
    let mut case = Case::start(true).await;
    let mut mcp = case.mcp().await;
    let id = case.send("I found the regression in parser.py.").await;
    mcp.read().await;
    case.submitted(&id).await;
    let prompts = [
        format!("<channel source=\"a2amx\">\n{ENVELOPE_TEXT} changed\n</channel>"),
        "<channel source=\"a2amx\">\nno envelope\n</channel>".into(),
        "<channel source=\"a2amx\">\n<a2amx-message id=\"m_999\">unknown</a2amx-message>\n</channel>".into(),
        format!("<channel source=\"a2amx\">\n{ENVELOPE_TEXT}\n<a2amx-message id=\"m_999\">another</a2amx-message>\n</channel>"),
    ];
    for prompt in prompts {
        let verdict = case.report(&prompt).await;
        assert!(
            matches!(verdict, Response::PromptVerdict { verdict, reason:None } if verdict == "allow")
        );
        assert_eq!(
            case.status(&id).await.evidence.as_deref(),
            Some("write_complete")
        );
    }
    let verdict = case.report("Human follow-up").await;
    assert!(
        matches!(verdict, Response::PromptVerdict { verdict, reason:None } if verdict == "allow")
    );
    let second = case.send("Second message").await;
    mcp.read().await;
    assert_eq!(
        case.submitted(&second).await.evidence.as_deref(),
        Some("write_complete")
    );
}

#[tokio::test]
async fn human_prompt_on_channel_session_allows_without_receipt() {
    let case = Case::start(true).await;
    let verdict = case.report("Human question").await;
    assert!(
        matches!(verdict, Response::PromptVerdict { verdict, reason:None } if verdict == "allow")
    );
}

#[test]
fn channel_launch_adds_development_flags_and_preserves_plain_launch() {
    for (authorize, count) in [(true, 10), (false, 8)] {
        let argv = wire_claude_channel_argv(
            vec!["claude".into(), "--".into(), "prompt".into()],
            Path::new("/usr/bin/a2amx"),
            Path::new("."),
            authorize,
            None,
        )
        .expect("Claude channel argv wiring");
        assert_eq!(argv.len() - 3, count);
        assert_eq!(argv[0], "claude");
        assert_eq!(argv[1], "--mcp-config");
        let config: Value = serde_json::from_str(&argv[2]).unwrap();
        assert_eq!(
            config,
            json!({"mcpServers":{"a2amx":{"command":"/usr/bin/a2amx","args":["mcp","--channel"]}}})
        );
        assert_eq!(
            &argv[3..7],
            [
                "--dangerously-load-development-channels",
                "server:a2amx",
                "--allowedTools",
                "mcp__a2amx__list_agents,mcp__a2amx__send_message,mcp__a2amx__message_status,mcp__a2amx__reset_session"
            ]
        );
        assert_eq!(argv[argv.len() - 4], "--settings");
        let settings: Value = serde_json::from_str(&argv[argv.len() - 3]).unwrap();
        assert_eq!(
            settings,
            json!({"hooks":{"UserPromptSubmit":[{"hooks":[{"type":"command","command":"'/usr/bin/a2amx' hook","timeout":5}]}]}})
        );
        assert_eq!(&argv[argv.len() - 2..], ["--", "prompt"]);
    }
    let plain = wire_claude_argv(
        vec!["claude".into()],
        Path::new("/usr/bin/a2amx"),
        Path::new("."),
        false,
        None,
    )
    .expect("Claude argv wiring");
    assert_eq!(plain.len(), 7);
    assert_eq!(plain[1], "--mcp-config");
    assert_eq!(
        serde_json::from_str::<Value>(&plain[2]).unwrap(),
        json!({"mcpServers":{"a2amx":{"command":"/usr/bin/a2amx","args":["mcp"]}}})
    );
    assert_eq!(
        &plain[3..6],
        [
            "--allowedTools",
            "mcp__a2amx__list_agents,mcp__a2amx__send_message,mcp__a2amx__message_status,mcp__a2amx__reset_session",
            "--settings"
        ]
    );
}

#[tokio::test]
async fn claude_cli_defaults_to_channel_and_no_channel_preserves_pty_route() {
    for no_channel in [false, true] {
        let (dir, _daemon) = common::start_daemon().await;
        let credentials = dir.path().join("launch-credentials");
        let args_file = dir.path().join("launch-args");
        let marker_file = dir.path().join("launch-marker");
        let mut command = Command::new(env!("CARGO_BIN_EXE_a2amx"));
        command
            .arg("--home")
            .arg(dir.path())
            .args(["new", "--detach", "--harness", "claude"]);
        if no_channel {
            command.arg("--no-channel");
        }
        let output = command.args(["--", "sh", "-c", "printf '%s' \"$A2AMX_TOKEN\" > \"$OUT\"; printf '%s\\n' \"$@\" > \"$ARGS\"; printf '%s' \"${A2AMX_CLAUDE_CHANNEL-unset}\" > \"$MARKER\"; sleep 30", "sh"])
            .env("OUT", &credentials).env("ARGS", &args_file).env("MARKER", &marker_file)
            .env_remove(CHANNEL_ENV).output().await.unwrap();
        assert!(output.status.success());
        let args = common::eventually(|| async {
            let path = args_file.clone();
            let text = tokio::task::spawn_blocking(move || std::fs::read_to_string(path))
                .await
                .ok()?
                .ok()?;
            (text.lines().count() >= 8).then_some(text)
        })
        .await;
        let args: Vec<&str> = args.lines().collect();
        let config: Value = serde_json::from_str(args[1]).unwrap();
        if no_channel {
            assert_eq!(args.len(), 8);
            assert_eq!(config["mcpServers"]["a2amx"]["args"], json!(["mcp"]));
        } else {
            assert_eq!(args.len(), 10);
            assert_eq!(
                config["mcpServers"]["a2amx"]["args"],
                json!(["mcp", "--channel"])
            );
        }
        let marker = tokio::task::spawn_blocking(move || std::fs::read_to_string(marker_file))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(marker, "unset");
        let token = tokio::task::spawn_blocking(move || std::fs::read_to_string(credentials))
            .await
            .unwrap()
            .unwrap();
        let client = Client::connect_addr(_daemon.addrs()[0], &token)
            .await
            .unwrap();
        if no_channel {
            assert_eq!(
                client.bridge().await.err().unwrap().to_string(),
                "session is not an omp session"
            );
        } else {
            let _link = client
                .bridge()
                .await
                .expect("default Claude has native bridge");
        }
    }
}

#[tokio::test]
async fn development_dialog_gets_exactly_one_enter_only_for_channel_sessions() {
    for channel in [true, false] {
        let (dir, _daemon) = common::start_daemon().await;
        let mut admin = Client::connect(dir.path()).await.unwrap();
        let output = dir.path().join("dialog-enter");
        let mut env = vec![("OUT".into(), output.to_string_lossy().into_owned())];
        if channel {
            env.push((CHANNEL_ENV.into(), "1".into()));
        }
        let response = admin.request(Request::NewSession {
            argv: vec!["sh".into(), "-c".into(), "printf 'I am using this for local development\\n'; read x; echo \"$x\" > \"$OUT\"; read x; echo \"$x\" >> \"$OUT\"; sleep 30".into()],
            cols:80, rows:24, cwd:None, env, reset:vec![], control_from:vec![], watch:vec![], name:Some("agent-review".into()), harness:Harness::Claude, deliver:Some(Deliver::Auto), heartbeat:None,
        }).await.unwrap();
        assert!(matches!(response, Response::Created { .. }));
        if channel {
            let text = common::eventually(|| async {
                let path = output.clone();
                tokio::task::spawn_blocking(move || std::fs::read_to_string(path))
                    .await
                    .ok()?
                    .ok()
                    .filter(|text| !text.is_empty())
            })
            .await;
            assert_eq!(text, "\n");
            tokio::time::sleep(Duration::from_secs(1)).await;
            let text = tokio::task::spawn_blocking(move || std::fs::read_to_string(output))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                text, "\n",
                "no second Enter while the marker remains visible"
            );
        } else {
            tokio::time::sleep(Duration::from_secs(3)).await;
            assert!(
                !tokio::task::spawn_blocking(move || output.exists())
                    .await
                    .unwrap()
            );
        }
    }
}
