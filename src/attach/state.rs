//! `impl AttachmentState`: async driver performing `Model` intents over daemon and terminal I/O.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use anyhow::anyhow;

use crate::client::Client;
use crate::lookup::request_sessions;
use crate::wire::ClientFrame;

use super::model::{Attached, Intent};
use super::{AttachOutcome, AttachmentState, Opened, open_attachment};

fn push_front_in_order(queue: &mut VecDeque<Intent>, intents: Vec<Intent>) {
    for intent in intents.into_iter().rev() {
        queue.push_front(intent);
    }
}

impl AttachmentState {
    pub(super) async fn perform_intents(
        &mut self,
        intents: Vec<Intent>,
    ) -> anyhow::Result<Option<AttachOutcome>> {
        let mut queue = VecDeque::from(intents);
        while let Some(intent) = queue.pop_front() {
            match intent {
                Intent::Forward(bytes) => self.send_frame(ClientFrame::Input(bytes)).await?,
                Intent::Scroll(scroll) => self.send_frame(ClientFrame::Scroll(scroll)).await?,
                Intent::Redraw => self.send_frame(ClientFrame::Redraw).await?,
                Intent::Release => self.send_frame(ClientFrame::Release).await?,
                Intent::SetViewer { viewer } => {
                    self.send_frame(ClientFrame::Mode { viewer }).await?;
                }
                Intent::Resize { cols, rows } => {
                    self.send_frame(ClientFrame::Resize { cols, rows }).await?;
                }
                Intent::Output(bytes) => self.output.send(bytes)?,
                Intent::RefreshPicker => {
                    let follow_up = self.refresh_picker().await?;
                    push_front_in_order(&mut queue, follow_up);
                }
                Intent::OpenPicker => {
                    let follow_up = self.open_picker().await?;
                    push_front_in_order(&mut queue, follow_up);
                }
                Intent::Switch { to } => {
                    let follow_up = self.switch_session(to).await?;
                    push_front_in_order(&mut queue, follow_up);
                }
                Intent::Detach => {
                    self.send_frame(ClientFrame::Detach).await?;
                    return Ok(Some(self.detached_outcome()));
                }
            }
        }
        // picker_control is Some only while model.in_picker()
        if !self.model.in_picker() {
            self.picker_control = None;
        }
        Ok(None)
    }

    async fn send_frame(&mut self, frame: ClientFrame) -> anyhow::Result<()> {
        if let Some(attached) = self.attachment.as_mut() {
            attached.send(frame).await?;
        }
        Ok(())
    }

    async fn open_picker(&mut self) -> anyhow::Result<Vec<Intent>> {
        let mut control = Client::connect(&self.home).await?;
        let sessions = request_sessions(&mut control).await?;
        self.picker_control = Some(control);
        Ok(self.model.picker_opened(sessions))
    }

    async fn refresh_picker(&mut self) -> anyhow::Result<Vec<Intent>> {
        let control = self
            .picker_control
            .as_mut()
            .ok_or_else(|| anyhow!("picker is not open"))?;
        let sessions = request_sessions(control).await?;
        Ok(self.model.picker_sessions(sessions))
    }

    async fn attach_session(&self, session: &str) -> anyhow::Result<Opened> {
        let client = Client::connect(&self.home).await?;
        open_attachment(
            client,
            &self.home,
            session,
            false,
            self.model.is_viewer(),
            self.model.viewport(),
        )
        .await
    }

    async fn switch_session(&mut self, selected: String) -> anyhow::Result<Vec<Intent>> {
        let current = self.model.session().to_owned();
        if let Some(attached) = self.attachment.as_mut() {
            let _ = attached.send(ClientFrame::Detach).await;
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break;
                }
                match tokio::time::timeout(remaining, attached.recv()).await {
                    Ok(Ok(Some(_))) => {}
                    Ok(Ok(None)) | Ok(Err(_)) | Err(_) => break,
                }
            }
        }
        self.attachment = None;
        match self.attach_session(&selected).await {
            Ok(opened) => {
                self.attachment = Some(opened.attachment);
                Ok(self.model.attached(
                    selected,
                    Attached {
                        in_team: opened.in_team,
                        granted_top: opened.top_rows,
                    },
                ))
            }
            Err(error) => match self.attach_session(&current).await {
                Ok(opened) => {
                    self.attachment = Some(opened.attachment);
                    Ok(self.model.switch_failed(
                        error.to_string(),
                        Attached {
                            in_team: opened.in_team,
                            granted_top: opened.top_rows,
                        },
                    ))
                }
                Err(restore_error) => Err(anyhow!(
                    "failed to switch to {selected}: {error}; failed to restore {current}: {restore_error}"
                )),
            },
        }
    }
}
