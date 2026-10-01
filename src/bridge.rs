//! Relay an OMP extension's JSON lines to the daemon bridge and MCP server.

use std::io::{self, Write};
use std::net::SocketAddr;
use std::thread;

use anyhow::{Result, anyhow, bail};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

use crate::client::{BridgeLink, Client};
use crate::mcp::{self, StdioEvent};
use crate::wire::{BridgeDown, BridgeUp, MAX_FRAME_LEN};

const ATTACHED_REFUSAL: &str = "a bridge is already attached to this session";
const NON_OMP_REFUSAL: &str = "session is not an omp session";

/// Run the OMP extension bridge relay on stdio.
pub async fn run() -> Result<()> {
    let (address, token) = mcp::configuration()?;
    let client = Client::connect_addr(address, &token).await?;
    let link = match client.bridge().await {
        Ok(link) => link,
        Err(error) => {
            let Some(reason) = attach_refusal(&error) else {
                return Err(error);
            };
            let output = Output::spawn()?;
            if let Err(write_error) = output.send_json(&BridgeDown::Refused {
                reason: reason.to_owned(),
            }) {
                let _ = output.finish().await;
                return Err(write_error);
            }
            return output.finish().await;
        }
    };

    // shortcut: the bridge does not reconnect by itself; add a bounded reconnect
    // with state replay only if respawning proves too slow.
    relay(link, address, token, Output::spawn()?).await
}

fn attach_refusal(error: &anyhow::Error) -> Option<&'static str> {
    for cause in error.chain() {
        if cause.to_string() == ATTACHED_REFUSAL {
            return Some(ATTACHED_REFUSAL);
        }
        if cause.to_string() == NON_OMP_REFUSAL {
            return Some(NON_OMP_REFUSAL);
        }
    }
    None
}

async fn relay(
    mut link: BridgeLink,
    address: SocketAddr,
    token: String,
    mut output: Output,
) -> Result<()> {
    let output_tx = output.sender()?;
    let error_tx = output.error_sender()?;
    let (requests_tx, requests_rx) = mpsc::unbounded_channel();
    let mcp_task = spawn_mcp_task(address, token, requests_rx, output_tx.clone(), error_tx);
    let mut lines = mcp::spawn_stdin_reader();

    let relay_result: Result<()> = loop {
        tokio::select! {
            event = lines.recv() => {
                match event {
                    Some(StdioEvent::Line(line)) => {
                        let input = match parse_extension_line(&line) {
                            Ok(input) => input,
                            Err(error) => break Err(error),
                        };
                        match input {
                            ExtensionInput::Bridge(frame) => {
                                if let Err(error) = link.send(frame).await {
                                    break Err(error);
                                }
                            }
                            ExtensionInput::Mcp(request) => {
                                if requests_tx.send(request).is_err() {
                                    break Err(anyhow!("MCP task stopped"));
                                }
                            }
                        }
                    }
                    Some(StdioEvent::Eof) | None => break Ok(()),
                    Some(StdioEvent::Error(message)) => {
                        break Err(anyhow!("reading bridge stdin: {message}"));
                    }
                }
            }
            error = output.errors.recv() => {
                match error {
                    Some(error) => break Err(anyhow!("bridge worker: {error}")),
                    None => break Err(anyhow!("bridge worker stopped")),
                }
            }
            frame = link.recv() => {
                match frame {
                    Ok(Some(frame)) => {
                        let refused = matches!(&frame, BridgeDown::Refused { .. });
                        if let Err(error) = send_json_to(&output_tx, &frame) {
                            break Err(error);
                        }
                        if refused {
                            break Ok(());
                        }
                    }
                    Ok(None) => break Err(anyhow!("daemon bridge connection closed")),
                    Err(error) => break Err(error),
                }
            }
        }
    };

    drop(requests_tx);
    mcp_task.abort();
    let _ = mcp_task.await;
    drop(output_tx);

    let finish_result = output.finish().await;
    match relay_result {
        Err(error) => Err(error),
        Ok(()) => finish_result,
    }
}

fn spawn_mcp_task(
    address: SocketAddr,
    token: String,
    mut requests: mpsc::UnboundedReceiver<Vec<u8>>,
    output: mpsc::UnboundedSender<WriterMessage>,
    errors: mpsc::UnboundedSender<String>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut server = mcp::McpServer::new(address, token);
        while let Some(request) = requests.recv().await {
            let Some(response) = server.handle_line(&request).await else {
                continue;
            };
            let value = json!({"type": "mcp", "response": response});
            let bytes = match serde_json::to_vec(&value) {
                Ok(bytes) => bytes,
                Err(_) => {
                    let _ = errors.send("cannot serialize MCP response".to_owned());
                    return;
                }
            };
            if bytes.len() > MAX_FRAME_LEN {
                let _ = errors.send("MCP response exceeds maximum length".to_owned());
                return;
            }
            if output.send(WriterMessage::Line(bytes)).is_err() {
                return;
            }
        }
    })
}

enum ExtensionInput {
    Bridge(BridgeUp),
    Mcp(Vec<u8>),
}

fn parse_extension_line(line: &[u8]) -> Result<ExtensionInput> {
    let payload = match line.last() {
        Some(b'\n') => &line[..line.len() - 1],
        _ => line,
    };
    validate_payload_len(payload.len())?;

    let value =
        serde_json::from_slice::<Value>(payload).map_err(|_| anyhow!("invalid bridge line"))?;
    let Some(kind) = value.get("type").and_then(Value::as_str) else {
        bail!("invalid bridge line");
    };
    if kind == "mcp" {
        let request = value
            .get("request")
            .ok_or_else(|| anyhow!("invalid bridge line"))?;
        let request = serde_json::to_vec(request)?;
        validate_payload_len(request.len())?;
        return Ok(ExtensionInput::Mcp(request));
    }

    let frame = serde_json::from_value(value).map_err(|_| anyhow!("invalid bridge line"))?;
    Ok(ExtensionInput::Bridge(frame))
}

fn validate_payload_len(length: usize) -> Result<()> {
    if length > MAX_FRAME_LEN {
        bail!("frame exceeds maximum length");
    }
    Ok(())
}

enum WriterMessage {
    Line(Vec<u8>),
    Flush(oneshot::Sender<io::Result<()>>),
}

struct Output {
    sender: Option<mpsc::UnboundedSender<WriterMessage>>,
    errors: mpsc::UnboundedReceiver<String>,
    error_sender: Option<mpsc::UnboundedSender<String>>,
    join: Option<thread::JoinHandle<()>>,
}

impl Output {
    fn spawn() -> Result<Self> {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let (error_sender, errors) = mpsc::unbounded_channel();
        let writer_errors = error_sender.clone();
        let join = thread::Builder::new()
            .name("a2amx-bridge-stdout".to_owned())
            .spawn(move || {
                let stdout = io::stdout();
                let mut stdout = stdout.lock();
                while let Some(message) = receiver.blocking_recv() {
                    match message {
                        WriterMessage::Line(bytes) => {
                            let result = (|| -> io::Result<()> {
                                stdout.write_all(&bytes)?;
                                stdout.write_all(b"\n")?;
                                stdout.flush()
                            })();
                            if let Err(error) = result {
                                let _ = writer_errors.send(error.to_string());
                                return;
                            }
                        }
                        WriterMessage::Flush(done) => {
                            let result = stdout.flush();
                            let failed = result.is_err();
                            if let Err(error) = &result {
                                let _ = writer_errors.send(error.to_string());
                            }
                            let _ = done.send(result);
                            if failed {
                                return;
                            }
                        }
                    }
                }
            })?;
        Ok(Self {
            sender: Some(sender),
            errors,
            error_sender: Some(error_sender),
            join: Some(join),
        })
    }

    fn sender(&self) -> Result<mpsc::UnboundedSender<WriterMessage>> {
        self.sender
            .as_ref()
            .cloned()
            .ok_or_else(|| anyhow!("stdout writer stopped"))
    }

    fn error_sender(&self) -> Result<mpsc::UnboundedSender<String>> {
        self.error_sender
            .as_ref()
            .cloned()
            .ok_or_else(|| anyhow!("bridge worker stopped"))
    }

    fn send_json<T: Serialize>(&self, value: &T) -> Result<()> {
        let sender = self
            .sender
            .as_ref()
            .ok_or_else(|| anyhow!("stdout writer stopped"))?;
        send_json_to(sender, value)
    }

    async fn finish(mut self) -> Result<()> {
        let sender = self
            .sender
            .take()
            .ok_or_else(|| anyhow!("stdout writer stopped"))?;
        let (done, flushed) = oneshot::channel();
        let flush_result = if sender.send(WriterMessage::Flush(done)).is_err() {
            Err(anyhow!("stdout writer stopped"))
        } else {
            tokio::select! {
                result = flushed => match result {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(error)) => Err(anyhow!("writing stdout: {error}")),
                    Err(_) => Err(anyhow!("stdout writer stopped")),
                },
                error = self.errors.recv() => match error {
                    Some(error) => Err(anyhow!("writing stdout: {error}")),
                    None => Err(anyhow!("stdout writer stopped")),
                },
            }
        };
        drop(sender);
        drop(self.error_sender.take());
        let join_result = self.join_writer().await;
        match flush_result {
            Err(error) => Err(error),
            Ok(()) => join_result,
        }
    }

    async fn join_writer(&mut self) -> Result<()> {
        let Some(join) = self.join.take() else {
            return Ok(());
        };
        tokio::task::spawn_blocking(move || join.join())
            .await
            .map_err(|_| anyhow!("stdout writer join failed"))?
            .map_err(|_| anyhow!("stdout writer thread panicked"))
    }
}

fn send_json_to<T: Serialize>(
    sender: &mpsc::UnboundedSender<WriterMessage>,
    value: &T,
) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    validate_payload_len(bytes.len())?;
    sender
        .send(WriterMessage::Line(bytes))
        .map_err(|_| anyhow!("stdout writer stopped"))
}
