//! The select loop over stdin, resize, the viewer tick and daemon frames.

use std::io::{self, Read};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Context;
use tokio::sync::mpsc;

use crate::client::Attachment;
use crate::terminal::terminal_size_with_default;
use crate::wire::{ClientFrame, ServerFrame};

use super::input::scroll_status;
use super::model::Mode;
use super::{AttachOutcome, AttachmentState, INPUT_CHUNK, MOUSE_CAPTURE};

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
                    return Ok(AttachOutcome {
                        message: format!("[detached from {}]", state.session),
                    });
                };
                if let Some(outcome) = state.handle_input(&bytes).await? {
                    return Ok(outcome);
                }
            }
            _ = sigwinch.recv() => {
                if let Ok((new_cols, new_rows)) = terminal_size_with_default() {
                    state.geometry.viewport.cols = new_cols;
                    state.geometry.viewport.rows = new_rows;
                    if let Mode::Picker(picker) = &mut state.mode {
                        picker.cols = new_cols;
                        picker.rows = new_rows;
                        picker.recompute_offset();
                        state.output.send(picker.render())?;
                    }
                    if let Some(attached) = state.attachment.as_mut() {
                        attached
                            .send(ClientFrame::Resize {
                                cols: new_cols,
                                rows: state.geometry.pty_rows(),
                            })
                            .await?;
                    }
                    if state.viewer.is_viewer() {
                        state.draw_status()?;
                        state.draw_viewer_decorations(Instant::now())?;
                    }
                }
            }
            _ = tick.tick() => {
                let now = Instant::now();
                if state.viewer.idle_expired(now, state.viewer_after) {
                    if let Some(attached) = state.attachment.as_mut() {
                        attached.send(ClientFrame::Mode { viewer: true }).await?;
                    }
                    state.draw_status()?;
                    state.draw_viewer_decorations(now)?;
                }
                if state.viewer.take_overlay_end(now) {
                    if let Some(attached) = state.attachment.as_mut() {
                        attached.send(ClientFrame::Redraw).await?;
                    }
                    state.draw_status()?;
                    state.draw_viewer_decorations(now)?;
                }
            }
            frame = recv_attachment(&mut state.attachment), if has_attachment => {
                let Some(frame) = frame else { continue; };
                let frame = frame?;
                let Some(frame) = frame else {
                    return Ok(AttachOutcome {
                        message: format!("[detached from {}]", state.session),
                    });
                };
                match frame {
                    ServerFrame::Data(data) => {
                        let has_mouse_mode = state.scan_session_mouse(&data);
                        if !matches!(state.mode, Mode::Picker(_)) {
                            state.output.send(data)?;
                            if has_mouse_mode && !state.session_mouse {
                                state.output.send(MOUSE_CAPTURE.to_vec())?;
                            }
                            state.draw_status()?;
                            if matches!(state.mode, Mode::Scroll(_)) {
                                state.output.send(scroll_status(
                                    state.geometry.viewport.cols,
                                    state.geometry.viewport.rows,
                                ))?;
                            }
                            state.draw_viewer_decorations(Instant::now())?;
                        }
                    }
                    ServerFrame::Status(info) => {
                        state.status = Some(info);
                        state.draw_status()?;
                        state.draw_viewer_decorations(Instant::now())?;
                    }
                    ServerFrame::Exit(code) => {
                        return Ok(AttachOutcome {
                            message: format!("[session {} exited with code {code}]", state.session),
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
