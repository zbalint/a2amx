//! `impl AttachmentState`: drawing, input handling, member navigation, session switching.

use std::time::{Duration, Instant};

use anyhow::anyhow;

use crate::client::{Attachment, Client};
use crate::emulator::Scroll;
use crate::lookup::request_sessions;
use crate::mouse::{self, MouseReport, Shifted};
use crate::picker::Outcome;
use crate::prefix::{Action, Command as PrefixCommand, PrefixMachine};
use crate::status;
use crate::viewer::{self, Viewer};
use crate::wire::ClientFrame;

use super::input::{ScrollKey, ScrollParser, find_mouse_report, is_focus_report, parse_decimal};
use super::model::{Geometry, Mode, Viewport};
use super::picker::PickerScreen;
use super::{
    AttachOutcome, AttachmentConfig, AttachmentState, MousePress, Opened, open_attachment,
};

impl AttachmentState {
    pub(super) fn new(attachment: Attachment, config: AttachmentConfig) -> Self {
        let AttachmentConfig {
            home,
            prefix,
            session,
            cols,
            rows,
            in_team,
            viewer_after,
            output,
        } = config;
        let top_rows = attachment.top_rows();
        Self {
            attachment: Some(attachment),
            home: home.to_owned(),
            session,
            geometry: Geometry {
                viewport: Viewport {
                    cols,
                    rows,
                    status_visible: true,
                },
                in_team,
                granted_top: top_rows,
            },
            team_regions: Vec::new(),
            mouse_press: None,
            prefix_machine: PrefixMachine::new(prefix),
            prefix_byte: prefix,
            after_prefix: false,
            status: None,
            session_mouse: false,
            mode: Mode::Live,
            picker_control: None,
            viewer: Viewer::new(Instant::now(), false),
            viewer_after,
            output,
        }
    }

    pub(super) fn draw_status(&mut self) -> anyhow::Result<()> {
        if !self.geometry.viewport.status_visible || !self.mode.is_live() {
            self.team_regions.clear();
            return Ok(());
        }
        let cols = self.geometry.viewport.cols;
        let rows = self.geometry.viewport.rows;
        let mut bytes = Vec::new();
        if self.geometry.granted_top > 0 {
            let line = self.status.as_ref().and_then(|info| {
                info.team.as_ref().map(|team| {
                    status::render_team_line(&self.session, &team.name, &info.members, cols)
                })
            });
            let line =
                line.unwrap_or_else(|| status::render_team_line(&self.session, "", &[], cols));
            self.team_regions = line.regions;
            bytes.extend(line.bytes);
            if self.geometry.granted_top == 2 {
                bytes.extend(status::render_team_rule(cols, true));
            }
        } else {
            self.team_regions.clear();
        }
        bytes.extend(status::render(
            &self.session,
            self.status.as_ref(),
            self.prefix_byte,
            cols,
            rows,
        ));
        self.output.send(bytes)?;
        Ok(())
    }
    pub(super) fn draw_viewer_decorations(&mut self, now: Instant) -> anyhow::Result<()> {
        if !self.viewer.is_viewer()
            || !self.geometry.viewport.status_visible
            || !self.mode.is_live()
        {
            return Ok(());
        }
        self.output.send(viewer::marker(
            self.geometry.viewport.cols,
            self.geometry.viewport.rows,
        ))?;
        if self.viewer.overlay_active(now) {
            self.output.send(viewer::overlay(
                self.geometry.viewport.cols,
                self.geometry.pty_rows(),
                self.geometry.granted_top,
                self.prefix_byte,
            ))?;
        }
        Ok(())
    }

    pub(super) async fn handle_input(
        &mut self,
        bytes: &[u8],
    ) -> anyhow::Result<Option<AttachOutcome>> {
        let now = Instant::now();
        if !is_focus_report(bytes) {
            self.viewer.own_input(now);
        }
        let mut offset = 0;
        while offset < bytes.len() {
            let session_mouse = self.session_mouse && !self.viewer.is_viewer();
            // Only live view with session mouse on and no team rows hands the raw bytes to the session.
            // shortcut: with top_rows > 0 and session_mouse, split input remains unshifted until a complete report is found.
            let live = self.mode.is_live();
            let pass_through = live && session_mouse && self.geometry.granted_top == 0;
            let report = (!pass_through)
                .then(|| find_mouse_report(bytes, offset))
                .flatten();
            if let Some((start, end)) = report
                && start == offset
            {
                let Some(parsed) = mouse::parse_report(bytes, offset) else {
                    return Err(anyhow!("mouse report parser lost a report boundary"));
                };
                if self.geometry.granted_top > 0 && self.mode.is_live() {
                    self.handle_mouse_report(parsed).await?;
                } else {
                    self.handle_mouse_button(parsed.button).await?;
                }
                offset = end;
                continue;
            }
            let segment_end = report.map_or(bytes.len(), |(start, _)| start);
            match &mut self.mode {
                Mode::Picker(_) => {
                    if let Some(outcome) = self
                        .handle_picker_input(&bytes[offset..offset + 1], bytes == [0x1b])
                        .await?
                    {
                        return Ok(Some(outcome));
                    }
                    offset += 1;
                }
                Mode::Scroll(_) => {
                    self.feed_scroll_input(&bytes[offset..offset + 1]).await?;
                    offset += 1;
                }
                Mode::Live => {
                    let count = self.prefix_batch_len(bytes, offset, segment_end);
                    let actions = self.prefix_machine.feed(&bytes[offset..offset + count]);
                    if let Some(outcome) = self.handle_prefix_actions(actions, now).await? {
                        return Ok(Some(outcome));
                    }
                    offset += count;
                }
            }
        }
        Ok(None)
    }

    async fn feed_scroll_input(&mut self, byte: &[u8]) -> anyhow::Result<()> {
        let Mode::Scroll(parser) = &mut self.mode else {
            return Ok(());
        };
        for key in parser.feed(byte) {
            match key {
                ScrollKey::Scroll(scroll) => {
                    if let Some(attached) = self.attachment.as_mut() {
                        attached.send(ClientFrame::Scroll(scroll)).await?;
                    }
                }
                ScrollKey::Quit => {
                    if let Some(attached) = self.attachment.as_mut() {
                        attached.send(ClientFrame::Scroll(Scroll::Bottom)).await?;
                        attached.send(ClientFrame::Redraw).await?;
                    }
                    self.mode = Mode::Live;
                }
            }
        }
        Ok(())
    }

    fn prefix_batch_len(&mut self, bytes: &[u8], offset: usize, segment_end: usize) -> usize {
        // End a batch at the prefix and consume its next byte separately:
        // a command can change how the remaining bytes must be interpreted.
        if self.after_prefix {
            self.after_prefix = false;
            1
        } else if let Some(index) = bytes[offset..segment_end]
            .iter()
            .position(|b| *b == self.prefix_byte)
        {
            self.after_prefix = true;
            index + 1
        } else {
            segment_end - offset
        }
    }

    async fn forward_or_block(&mut self, input: Vec<u8>, now: Instant) -> anyhow::Result<()> {
        if self.viewer.is_viewer() {
            if !is_focus_report(&input) {
                self.viewer.blocked(now);
                self.draw_viewer_decorations(now)?;
            }
        } else if let Some(attached) = self.attachment.as_mut() {
            attached.send(ClientFrame::Input(input)).await?;
        }
        Ok(())
    }

    async fn toggle_viewer_mode(&mut self, now: Instant) -> anyhow::Result<()> {
        let viewer = self.viewer.toggle(now);
        if let Some(attached) = self.attachment.as_mut() {
            attached.send(ClientFrame::Mode { viewer }).await?;
            if !viewer {
                attached
                    .send(ClientFrame::Resize {
                        cols: self.geometry.viewport.cols,
                        rows: self.geometry.pty_rows(),
                    })
                    .await?;
                attached.send(ClientFrame::Redraw).await?;
            }
        }
        self.draw_status()?;
        self.draw_viewer_decorations(now)?;
        Ok(())
    }

    async fn handle_prefix_actions(
        &mut self,
        actions: Vec<Action>,
        now: Instant,
    ) -> anyhow::Result<Option<AttachOutcome>> {
        for action in actions {
            match action {
                Action::Forward(input) => self.forward_or_block(input, now).await?,
                Action::Command(PrefixCommand::Detach) => {
                    if let Some(attached) = self.attachment.as_mut() {
                        attached.send(ClientFrame::Detach).await?;
                    }
                    return Ok(Some(AttachOutcome {
                        message: format!("[detached from {}]", self.session),
                    }));
                }
                Action::Command(PrefixCommand::ScrollMode) => {
                    self.enter_scroll_mode().await?;
                }
                Action::Command(PrefixCommand::SessionPicker) => {
                    self.open_picker().await?;
                }
                Action::Command(PrefixCommand::StatusLine) => {
                    self.toggle_status_line().await?;
                }
                Action::Command(PrefixCommand::ViewerMode) => {
                    self.toggle_viewer_mode(now).await?;
                }
                Action::Command(PrefixCommand::TeamMember(number)) => {
                    if let Some(id) = self.member_by_number(number) {
                        self.select_member(id).await?;
                    }
                }
                Action::Command(PrefixCommand::TeamNext) => {
                    if let Some(id) = self.next_member(false) {
                        self.select_member(id).await?;
                    }
                }
                Action::Command(PrefixCommand::TeamPrev) => {
                    if let Some(id) = self.next_member(true) {
                        self.select_member(id).await?;
                    }
                }
                Action::Command(PrefixCommand::Release) => {
                    if !self.viewer.is_viewer()
                        && let Some(attached) = self.attachment.as_mut()
                    {
                        attached.send(ClientFrame::Release).await?;
                    }
                }
            }
        }
        Ok(None)
    }

    async fn open_picker(&mut self) -> anyhow::Result<()> {
        let mut control = Client::connect(&self.home).await?;
        let sessions = request_sessions(&mut control).await?;
        let screen = PickerScreen::new(
            sessions,
            &self.session,
            self.geometry.viewport.cols,
            self.geometry.viewport.rows,
        );
        self.output.send(screen.render())?;
        self.team_regions.clear();
        self.mode = Mode::Picker(Box::new(screen));
        self.picker_control = Some(control);
        Ok(())
    }

    fn close_picker(&mut self) {
        self.mode = Mode::Live;
        self.picker_control = None;
    }

    async fn refresh_picker(&mut self) -> anyhow::Result<()> {
        let (Mode::Picker(screen), Some(control)) = (&mut self.mode, self.picker_control.as_mut())
        else {
            return Err(anyhow!("picker is not open"));
        };
        let sessions = request_sessions(control).await?;
        screen.replace_sessions(sessions);
        Ok(())
    }

    fn member_by_number(&self, number: u8) -> Option<String> {
        (self.geometry.granted_top > 0)
            .then(|| {
                self.status
                    .as_ref()
                    .and_then(|status| status.members.get(usize::from(number.saturating_sub(1))))
                    .map(|member| member.id.clone())
            })
            .flatten()
    }

    fn next_member(&self, previous: bool) -> Option<String> {
        if self.geometry.granted_top == 0 {
            return None;
        }
        let members = &self.status.as_ref()?.members;
        if members.is_empty() {
            return None;
        }
        let current = members.iter().position(|member| member.id == self.session);
        let start = current.unwrap_or(if previous { 0 } else { members.len() - 1 });
        for offset in 1..=members.len() {
            let index = if previous {
                (start + members.len() - offset) % members.len()
            } else {
                (start + offset) % members.len()
            };
            if !members[index].exited {
                return Some(members[index].id.clone());
            }
        }
        None
    }

    async fn select_member(&mut self, selected: String) -> anyhow::Result<()> {
        if self.geometry.granted_top == 0
            || selected == self.session
            || self
                .status
                .as_ref()
                .and_then(|status| status.members.iter().find(|member| member.id == selected))
                .is_none_or(|member| member.exited)
        {
            return Ok(());
        }
        self.mouse_press = None;
        let _ = self.switch_session(selected).await?;
        Ok(())
    }

    async fn toggle_status_line(&mut self) -> anyhow::Result<()> {
        let visible = !self.geometry.viewport.status_visible;
        let cols = self.geometry.viewport.cols;
        let rows = self.geometry.viewport.rows;
        if !visible {
            if self.geometry.granted_top > 0 {
                let clear = status::render_team_line(&self.session, "", &[], cols);
                self.output.send(clear.bytes)?;
                if self.geometry.granted_top == 2 {
                    self.output.send(status::render_team_rule(cols, false))?;
                }
                self.team_regions.clear();
            }
            self.output.send(status::clear_status(cols, rows))?;
        }
        self.geometry.viewport.status_visible = visible;
        let desired_top = self.geometry.desired_top();
        if desired_top != self.geometry.granted_top {
            self.mouse_press = None;
            let _ = self.switch_session(self.session.clone()).await?;
        } else if self.viewer.is_viewer() {
            if let Some(attached) = self.attachment.as_mut() {
                attached
                    .send(ClientFrame::Resize {
                        cols,
                        rows: self.geometry.pty_rows(),
                    })
                    .await?;
            }
            self.draw_status()?;
            self.draw_viewer_decorations(Instant::now())?;
        } else if let Some(attached) = self.attachment.as_mut() {
            attached
                .send(ClientFrame::Resize {
                    cols,
                    rows: self.geometry.pty_rows(),
                })
                .await?;
        }
        Ok(())
    }

    async fn enter_scroll_mode(&mut self) -> anyhow::Result<()> {
        self.mode = Mode::Scroll(ScrollParser::default());
        if let Some(attached) = self.attachment.as_mut() {
            attached.send(ClientFrame::Redraw).await?;
        }
        Ok(())
    }

    async fn handle_mouse_button(&mut self, button: u16) -> anyhow::Result<()> {
        if matches!(self.mode, Mode::Picker(_)) {
            return Ok(());
        }
        match button {
            64 => {
                if !matches!(self.mode, Mode::Scroll(_)) {
                    self.enter_scroll_mode().await?;
                }
                // shortcut: the wheel uses a fixed three-line step; make it configurable only if asked.
                for _ in 0..3 {
                    if let Some(attached) = self.attachment.as_mut() {
                        attached.send(ClientFrame::Scroll(Scroll::LineUp)).await?;
                    }
                }
            }
            65 if matches!(self.mode, Mode::Scroll(_)) => {
                // shortcut: the wheel uses a fixed three-line step; make it configurable only if asked.
                for _ in 0..3 {
                    if let Some(attached) = self.attachment.as_mut() {
                        attached.send(ClientFrame::Scroll(Scroll::LineDown)).await?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    async fn handle_mouse_report(&mut self, report: MouseReport<'_>) -> anyhow::Result<()> {
        let session_mouse = self.session_mouse && !self.viewer.is_viewer();
        match mouse::shift_row(&report, self.geometry.granted_top) {
            Shifted::Claimed(claimed) => {
                if claimed.release {
                    match self.mouse_press.take() {
                        Some(MousePress::Team) if claimed.row == 1 => {
                            self.team_click(claimed.column).await?
                        }
                        Some(MousePress::Grid) if session_mouse => {
                            self.send_session_mouse(mouse::with_row(&claimed, 1))
                                .await?;
                        }
                        _ => {}
                    }
                } else if claimed.button == 0 {
                    self.mouse_press = Some(match claimed.row {
                        1 => MousePress::Team,
                        2 => MousePress::Rule,
                        _ => return Ok(()),
                    });
                }
            }
            Shifted::Grid(bytes) => {
                if report.release {
                    match self.mouse_press.take() {
                        Some(MousePress::Team) => {}
                        Some(MousePress::Grid) | None if session_mouse => {
                            self.send_session_mouse(bytes).await?;
                        }
                        _ => {}
                    }
                } else {
                    if report.button == 0 {
                        self.mouse_press = Some(MousePress::Grid);
                    }
                    if session_mouse {
                        self.send_session_mouse(bytes).await?;
                    } else {
                        self.handle_mouse_button(report.button).await?;
                    }
                }
            }
        }
        Ok(())
    }

    async fn send_session_mouse(&mut self, bytes: Vec<u8>) -> anyhow::Result<()> {
        if let Some(attached) = self.attachment.as_mut() {
            attached.send(ClientFrame::Input(bytes)).await?;
        }
        Ok(())
    }

    async fn team_click(&mut self, column: u16) -> anyhow::Result<()> {
        let target = self
            .team_regions
            .iter()
            .find(|region| region.first_col <= column && column <= region.last_col)
            .map(|region| region.target.clone());
        match target {
            Some(status::TeamTarget::Picker) => self.open_picker().await?,
            Some(status::TeamTarget::Session(id)) => self.select_member(id).await?,
            None => {}
        }
        Ok(())
    }

    pub(super) fn scan_session_mouse(&mut self, bytes: &[u8]) -> bool {
        let mut found = false;
        let mut offset = 0;
        while offset + 3 < bytes.len() {
            if bytes[offset] == 0x1b
                && bytes.get(offset + 1) == Some(&b'[')
                && bytes.get(offset + 2) == Some(&b'?')
            {
                let first = offset + 3;
                let mut end = first;
                while end < bytes.len() && bytes[end].is_ascii_digit() {
                    end += 1;
                }
                if end > first && matches!(bytes.get(end), Some(b'h' | b'l')) {
                    let mode = parse_decimal(&bytes[first..end]);
                    if matches!(mode, 1000 | 1002 | 1003) {
                        found = true;
                        self.session_mouse = bytes[end] == b'h';
                    }
                }
            }
            offset += 1;
        }
        found
    }

    async fn attach_session(&self, session: &str) -> anyhow::Result<Opened> {
        let client = Client::connect(&self.home).await?;
        open_attachment(
            client,
            &self.home,
            session,
            false,
            self.viewer.is_viewer(),
            self.geometry.viewport,
        )
        .await
    }

    fn adopt(&mut self, opened: Opened) {
        self.attachment = Some(opened.attachment);
        self.geometry.in_team = opened.in_team;
        self.geometry.granted_top = opened.top_rows;
        self.session_mouse = false;
        self.status = None;
    }

    async fn switch_session(&mut self, selected: String) -> anyhow::Result<bool> {
        let current = self.session.clone();
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
        self.mouse_press = None;
        let result = self.attach_session(&selected).await;
        match result {
            Ok(opened) => {
                self.adopt(opened);
                self.session = selected;
                self.team_regions.clear();
                self.output
                    .send(format!("\x1b]2;a2amx: {}\x07", self.session).into_bytes())?;
                self.close_picker();
                Ok(true)
            }
            // shortcut: a failed switch is not shown; add a bar notice if asked.
            Err(error) => match self.attach_session(&current).await {
                Ok(opened) => {
                    self.adopt(opened);
                    if let Mode::Picker(screen) = &mut self.mode {
                        screen.footer = Some(error.to_string());
                        self.refresh_picker().await?;
                        if let Mode::Picker(screen) = &self.mode {
                            self.output.send(screen.render())?;
                        }
                    }
                    Ok(false)
                }
                Err(restore_error) => Err(anyhow!(
                    "failed to switch to {selected}: {error}; failed to restore {current}: {restore_error}"
                )),
            },
        }
    }

    async fn handle_picker_input(
        &mut self,
        bytes: &[u8],
        escape_alone: bool,
    ) -> anyhow::Result<Option<AttachOutcome>> {
        let had_footer = {
            let Mode::Picker(screen) = &mut self.mode else {
                return Ok(None);
            };
            screen.footer.take().is_some()
        };
        if had_footer && let Mode::Picker(screen) = &self.mode {
            self.output.send(screen.render())?;
        }
        let actions = match &mut self.mode {
            Mode::Picker(screen) => screen.feed(bytes, escape_alone),
            _ => return Ok(None),
        };
        for action in actions {
            let outcome = {
                let Mode::Picker(screen) = &mut self.mode else {
                    return Ok(None);
                };
                let outcome = screen.apply(action);
                screen.parser.set_filter_mode(screen.view.filter_mode());
                outcome
            };
            match outcome {
                None => {
                    if let Mode::Picker(screen) = &self.mode {
                        self.output.send(screen.render())?;
                    }
                }
                Some(Outcome::Cancel) => {
                    if let Some(attached) = self.attachment.as_mut() {
                        attached.send(ClientFrame::Redraw).await?;
                    }
                    self.close_picker();
                    return Ok(None);
                }
                Some(Outcome::Attach(selected)) => {
                    if selected == self.session {
                        if let Some(attached) = self.attachment.as_mut() {
                            attached.send(ClientFrame::Redraw).await?;
                        }
                        self.close_picker();
                        return Ok(None);
                    }
                    if self.switch_session(selected).await? {
                        return Ok(None);
                    }
                }
            }
        }
        Ok(None)
    }
}
