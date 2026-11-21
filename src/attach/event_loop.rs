//! The select loop over stdin, resize, the viewer tick and daemon frames.

use std::io::{self, Read};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Context;
use tokio::sync::mpsc;

use crate::client::Attachment;
use crate::terminal::terminal_size_with_default;
use crate::wire::{ClientFrame, ServerFrame};

use super::{AttachOutcome, AttachmentState, INPUT_CHUNK};

pub(super) async fn run_attachment_loop(
    mut state: AttachmentState,
) -> anyhow::Result<AttachOutcome> {
    let (input_tx, mut input_rx) = mpsc::channel::<Option<Vec<u8>>>(8);
    spawn_stdin_reader(input_tx)?;
    let mut sigwinch =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::window_change())?;
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        let has_attachment = state.attachment.is_some();
        tokio::select! {
            input = input_rx.recv() => {
                let Some(bytes) = input.flatten() else {
                    if let Some(attached) = state.attachment.as_mut() {
                        let _ = attached.send(ClientFrame::Detach).await;
                    }
                    return Ok(state.detached_outcome());
                };
                let now = Instant::now();
                let mut offset = 0;
                while offset < bytes.len() {
                    let step = state.model.step(&bytes, offset, now);
                    offset += step.consumed;
                    if let Some(outcome) = state.perform_intents(step.intents).await? {
                        return Ok(outcome);
                    }
                }
            }
            _ = sigwinch.recv() => {
                if let Ok((new_cols, new_rows)) = terminal_size_with_default() {
                    let intents = state.model.resize(new_cols, new_rows, Instant::now());
                    if let Some(outcome) = state.perform_intents(intents).await? {
                        return Ok(outcome);
                    }
                }
            }
            _ = tick.tick() => {
                let intents = state.model.on_tick(Instant::now());
                if let Some(outcome) = state.perform_intents(intents).await? {
                    return Ok(outcome);
                }
            }
            frame = recv_attachment(&mut state.attachment), if has_attachment => {
                let Some(frame) = frame else { continue; };
                let frame = frame?;
                let Some(frame) = frame else {
                    return Ok(state.detached_outcome());
                };
                match frame {
                    ServerFrame::Data(data) => {
                        let intents = state.model.on_output(data, Instant::now());
                        if let Some(outcome) = state.perform_intents(intents).await? {
                            return Ok(outcome);
                        }
                    }
                    ServerFrame::Status(info) => {
                        let intents = state.model.on_status(info, Instant::now());
                        if let Some(outcome) = state.perform_intents(intents).await? {
                            return Ok(outcome);
                        }
                    }
                    ServerFrame::Exit(code) => {
                        return Ok(AttachOutcome {
                            message: format!("[session {} exited with code {code}]", state.model.session()),
                        });
                    }
                    ServerFrame::Detached(reason) => {
                        return Ok(AttachOutcome {
                            message: format!("[detached: {reason}]"),
                        });
                    }
                }
            }
        }
        let output = state.output.clone();
        tokio::task::spawn_blocking(move || output.flush()).await??;
    }
}

async fn recv_attachment(
    attachment: &mut Option<Attachment>,
) -> Option<anyhow::Result<Option<ServerFrame>>> {
    let attachment = attachment.as_mut()?;
    Some(attachment.recv().await)
}

fn spawn_stdin_reader(sender: mpsc::Sender<Option<Vec<u8>>>) -> anyhow::Result<()> {
    thread::Builder::new()
        .name("a2amx-stdin-reader".to_owned())
        .spawn(move || {
            let stdin = io::stdin();
            let mut stdin = stdin.lock();
            let mut buf = [0u8; INPUT_CHUNK];
            loop {
                match stdin.read(&mut buf) {
                    Ok(0) => {
                        let _ = sender.blocking_send(None);
                        return;
                    }
                    Ok(len) => {
                        if sender.blocking_send(Some(buf[..len].to_vec())).is_err() {
                            return;
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => {
                        let _ = sender.blocking_send(None);
                        return;
                    }
                }
            }
        })
        .context("start stdin reader")?;
    Ok(())
}
