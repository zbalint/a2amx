//! Pure synchronous model for the attach screen's state, input stepping, and intents.

use std::time::{Duration, Instant};

use crate::emulator::Scroll;
use crate::mouse::{self, MouseReport, Shifted};
use crate::picker::{Outcome, PickerAction};
use crate::prefix::{Action, Command as PrefixCommand, PrefixMachine};
use crate::status;
use crate::viewer::{self, Viewer};
use crate::wire::{SessionSummary, StatusInfo};

use super::input::{
    ScrollKey, ScrollParser, find_mouse_report, is_focus_report, parse_decimal, scroll_status,
};
use super::picker::PickerScreen;

const MOUSE_CAPTURE: &[u8] = b"\x1b[?1000h\x1b[?1006h";

/// What the human's terminal currently looks like, and whether the status line is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Viewport {
    pub cols: u16,
    pub rows: u16,
    pub status_visible: bool,
}

impl Viewport {
    /// Rows the session's PTY gets: the terminal rows minus the status rows and the granted top rows, at least 1.
    pub fn pty_rows(&self, top: u16) -> u16 {
        let status_rows = if self.status_visible && self.rows >= 4 {
            2
        } else if self.status_visible && self.rows == 3 {
            1
        } else {
            0
        };
        self.rows
            .saturating_sub(status_rows)
            .saturating_sub(top)
            .max(1)
    }

    /// Top rows the team line wants for this viewport: 2 at six rows or more, 1 at exactly five, else 0; 0 when
    /// the status line is hidden or the session is not in a team.
    pub fn desired_top(&self, in_team: bool) -> u16 {
        if !self.status_visible || !in_team {
            return 0;
        }
        if self.rows >= 6 {
            2
        } else if self.rows == 5 {
            1
        } else {
            0
        }
    }
}

/// The viewport plus what the daemon negotiated for the current session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub viewport: Viewport,
    pub in_team: bool,
    pub granted_top: u16,
}

impl Geometry {
    pub fn pty_rows(&self) -> u16 {
        self.viewport.pty_rows(self.granted_top)
    }

    pub fn desired_top(&self) -> u16 {
        self.viewport.desired_top(self.in_team)
    }
}

/// Exactly one input mode at a time.
enum Mode {
    Live,
    Scroll(ScrollParser),
    Picker(Box<PickerScreen>),
}

impl Mode {
    fn is_live(&self) -> bool {
        matches!(self, Mode::Live)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MousePress {
    Team,
    Rule,
    Grid,
}

/// The result of one synchronous `Model::step` call.
#[derive(Debug, PartialEq)]
pub struct Step {
    pub consumed: usize,
    pub intents: Vec<Intent>,
}

/// Side-effect intents produced by `Model` for the async driver to perform;
/// a barrier intent (`OpenPicker`, `Switch`, `Detach`) is always the last intent of a call; the driver relies on it.
#[derive(Debug, PartialEq)]
pub enum Intent {
    Forward(Vec<u8>),
    Scroll(Scroll),
    Redraw,
    Release,
    SetViewer { viewer: bool },
    Resize { cols: u16, rows: u16 },
    Output(Vec<u8>),
    RefreshPicker,
    OpenPicker,
    Switch { to: String },
    Detach,
}

/// Daemon negotiation result reported back to `Model` after attaching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attached {
    pub in_team: bool,
    pub granted_top: u16,
}

/// Configuration for constructing a [`Model`].
pub struct ModelConfig {
    pub session: String,
    pub geometry: Geometry,
    pub prefix: u8,
    pub viewer: bool,
    pub viewer_after: Option<Duration>,
}

/// Pure synchronous state machine for the attach screen.
pub struct Model {
    session: String,
    geometry: Geometry,
    mode: Mode,
    prefix_machine: PrefixMachine,
    prefix_byte: u8,
    mouse_press: Option<MousePress>,
    session_mouse: bool,
    viewer: Viewer,
    viewer_after: Option<Duration>,
    status: Option<StatusInfo>,
    team_regions: Vec<status::TeamRegion>,
}

impl Model {
    pub fn new(config: ModelConfig, now: Instant) -> Self {
        Self {
            session: config.session,
            geometry: config.geometry,
            mode: Mode::Live,
            prefix_machine: PrefixMachine::new(config.prefix),
            prefix_byte: config.prefix,
            mouse_press: None,
            session_mouse: false,
            viewer: Viewer::new(now, config.viewer),
            viewer_after: config.viewer_after,
            status: None,
            team_regions: Vec::new(),
        }
    }

    pub fn session(&self) -> &str {
        &self.session
    }

    pub fn is_viewer(&self) -> bool {
        self.viewer.is_viewer()
    }

    pub fn viewport(&self) -> Viewport {
        self.geometry.viewport
    }

    pub fn in_picker(&self) -> bool {
        matches!(self.mode, Mode::Picker(_))
    }

    /// Process one segment of `read` starting at `offset` and return the bytes consumed and intents.
    /// `offset == 0` starts a new read (`own_input` runs once per read).
    pub fn step(&mut self, read: &[u8], offset: usize, now: Instant) -> Step {
        if offset >= read.len() {
            return Step {
                consumed: 0,
                intents: Vec::new(),
            };
        }
        if offset == 0 && !is_focus_report(read) {
            self.viewer.own_input(now);
        }
        let session_mouse = self.session_mouse && !self.viewer.is_viewer();
        // Only live view with session mouse on and no team rows hands the raw bytes to the session.
        // shortcut: with top_rows > 0 and session_mouse, split input remains unshifted until a complete report is found.
        let live = self.mode.is_live();
        let pass_through = live && session_mouse && self.geometry.granted_top == 0;
        let report = (!pass_through)
            .then(|| find_mouse_report(read, offset))
            .flatten();
        if let Some(step) = self.handle_step_mouse_report(report, offset) {
            return step;
        }
        let segment_end = report.map_or(read.len(), |(start, _)| start);
        self.dispatch_mode(&read[offset..segment_end], read == [0x1b], now)
    }

    fn handle_step_mouse_report(
        &mut self,
        report: Option<(usize, MouseReport<'_>)>,
        offset: usize,
    ) -> Option<Step> {
        let (start, report) = report?;
        if start != offset {
            return None;
        }
        let end = report.end;
        let mut intents = Vec::new();
        if self.geometry.granted_top > 0 && self.mode.is_live() {
            self.handle_mouse_report(report, &mut intents);
        } else {
            self.handle_mouse_button(report.button, &mut intents);
        }
        Some(Step {
            consumed: end - offset,
            intents,
        })
    }

    fn dispatch_mode(&mut self, segment: &[u8], escape_alone: bool, now: Instant) -> Step {
        let mut intents = Vec::new();
        match &mut self.mode {
            Mode::Picker(_) => {
                self.handle_picker_input(&segment[..1], escape_alone, &mut intents);
                Step {
                    consumed: 1,
                    intents,
                }
            }
            Mode::Scroll(_) => {
                self.feed_scroll_input(&segment[..1], &mut intents);
                Step {
                    consumed: 1,
                    intents,
                }
            }
            Mode::Live => {
                let (consumed, actions) = self.prefix_machine.feed_until_command(segment);
                self.handle_prefix_actions(actions, now, &mut intents);
                Step { consumed, intents }
            }
        }
    }

    /// Scan daemon output for mouse-mode toggles and emit terminal `Output` intents.
    pub fn on_output(&mut self, data: Vec<u8>, now: Instant) -> Vec<Intent> {
        let has_mouse_mode = self.scan_session_mouse(&data);
        if matches!(self.mode, Mode::Picker(_)) {
            return Vec::new();
        }
        let mut intents = vec![Intent::Output(data)];
        if has_mouse_mode && !self.session_mouse {
            intents.push(Intent::Output(MOUSE_CAPTURE.to_vec()));
        }
        self.draw_status(&mut intents);
        if matches!(self.mode, Mode::Scroll(_)) {
            intents.push(Intent::Output(scroll_status(
                self.geometry.viewport.cols,
                self.geometry.viewport.rows,
            )));
        }
        self.draw_viewer_decorations(now, &mut intents);
        intents
    }

    /// Update status info and emit status/viewer `Output` intents.
    pub fn on_status(&mut self, info: StatusInfo, now: Instant) -> Vec<Intent> {
        self.status = Some(info);
        let mut intents = Vec::new();
        self.draw_status(&mut intents);
        self.draw_viewer_decorations(now, &mut intents);
        intents
    }

    /// Advance idle-viewer and overlay timers at `now`.
    pub fn on_tick(&mut self, now: Instant) -> Vec<Intent> {
        let mut intents = Vec::new();
        if self.viewer.idle_expired(now, self.viewer_after) {
            intents.push(Intent::SetViewer { viewer: true });
            self.draw_status(&mut intents);
            self.draw_viewer_decorations(now, &mut intents);
        }
        if self.viewer.take_overlay_end(now) {
            intents.push(Intent::Redraw);
            self.draw_status(&mut intents);
            self.draw_viewer_decorations(now, &mut intents);
        }
        intents
    }

    /// Handle a terminal resize to `(cols, rows)` at `now`.
    pub fn resize(&mut self, cols: u16, rows: u16, now: Instant) -> Vec<Intent> {
        self.geometry.viewport.cols = cols;
        self.geometry.viewport.rows = rows;
        let mut intents = Vec::new();
        if let Mode::Picker(picker) = &mut self.mode {
            picker.cols = cols;
            picker.rows = rows;
            picker.recompute_offset();
            intents.push(Intent::Output(picker.render()));
        }
        intents.push(Intent::Resize {
            cols,
            rows: self.geometry.pty_rows(),
        });
        if self.viewer.is_viewer() {
            self.draw_status(&mut intents);
            self.draw_viewer_decorations(now, &mut intents);
        }
        intents
    }

    /// Enter picker mode after the driver fetched the initial session list.
    pub fn picker_opened(&mut self, sessions: Vec<SessionSummary>) -> Vec<Intent> {
        let screen = PickerScreen::new(
            sessions,
            &self.session,
            self.geometry.viewport.cols,
            self.geometry.viewport.rows,
        );
        let rendered = screen.render();
        self.team_regions.clear();
        self.mode = Mode::Picker(Box::new(screen));
        vec![Intent::Output(rendered)]
    }

    /// Update the open picker's session list and re-render.
    pub fn picker_sessions(&mut self, sessions: Vec<SessionSummary>) -> Vec<Intent> {
        let Mode::Picker(screen) = &mut self.mode else {
            return Vec::new();
        };
        screen.replace_sessions(sessions);
        vec![Intent::Output(screen.render())]
    }

    /// Apply a completed session attach (or same-session reattach) and emit the title update.
    pub fn attached(&mut self, to: String, result: Attached) -> Vec<Intent> {
        self.adopt(result);
        self.session = to;
        self.team_regions.clear();
        self.mode = Mode::Live;
        vec![Intent::Output(
            format!("\x1b]2;a2amx: {}\x07", self.session).into_bytes(),
        )]
    }

    /// Restore the previous session after a failed switch and request a picker refresh if open.
    // shortcut: a failed switch is not shown; add a bar notice if asked.
    pub fn switch_failed(&mut self, error: String, restored: Attached) -> Vec<Intent> {
        self.adopt(restored);
        if let Mode::Picker(screen) = &mut self.mode {
            screen.footer = Some(error);
            vec![Intent::RefreshPicker]
        } else {
            Vec::new()
        }
    }

    fn adopt(&mut self, result: Attached) {
        self.geometry.in_team = result.in_team;
        self.geometry.granted_top = result.granted_top;
        self.session_mouse = false;
        self.status = None;
    }

    fn draw_status(&mut self, intents: &mut Vec<Intent>) {
        if !self.geometry.viewport.status_visible || !self.mode.is_live() {
            self.team_regions.clear();
            return;
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
            &status::StatusBar::new(&self.session, self.status.as_ref(), self.prefix_byte),
            cols,
            rows,
        ));
        intents.push(Intent::Output(bytes));
    }

    fn viewer_decorations_visible(&self) -> bool {
        self.viewer.is_viewer() && self.geometry.viewport.status_visible && self.mode.is_live()
    }

    fn draw_viewer_decorations(&self, now: Instant, intents: &mut Vec<Intent>) {
        if !self.viewer_decorations_visible() {
            return;
        }
        intents.push(Intent::Output(viewer::marker(
            self.geometry.viewport.cols,
            self.geometry.viewport.rows,
        )));
        if self.viewer.overlay_active(now) {
            intents.push(Intent::Output(viewer::overlay(
                self.geometry.viewport.cols,
                self.geometry.pty_rows(),
                self.geometry.granted_top,
                self.prefix_byte,
            )));
        }
    }

    fn feed_scroll_input(&mut self, byte: &[u8], intents: &mut Vec<Intent>) {
        let Mode::Scroll(parser) = &mut self.mode else {
            return;
        };
        for key in parser.feed(byte) {
            match key {
                ScrollKey::Scroll(scroll) => intents.push(Intent::Scroll(scroll)),
                ScrollKey::Quit => {
                    intents.push(Intent::Scroll(Scroll::Bottom));
                    intents.push(Intent::Redraw);
                    self.mode = Mode::Live;
                }
            }
        }
    }

    fn forward_or_block(&mut self, input: Vec<u8>, now: Instant, intents: &mut Vec<Intent>) {
        if self.viewer.is_viewer() {
            if !is_focus_report(&input) {
                self.viewer.blocked(now);
                self.draw_viewer_decorations(now, intents);
            }
        } else {
            intents.push(Intent::Forward(input));
        }
    }

    fn toggle_viewer_mode(&mut self, now: Instant, intents: &mut Vec<Intent>) {
        let viewer = self.viewer.toggle(now);
        intents.push(Intent::SetViewer { viewer });
        if !viewer {
            intents.push(Intent::Resize {
                cols: self.geometry.viewport.cols,
                rows: self.geometry.pty_rows(),
            });
            intents.push(Intent::Redraw);
        }
        self.draw_status(intents);
        self.draw_viewer_decorations(now, intents);
    }

    fn handle_prefix_actions(
        &mut self,
        actions: Vec<Action>,
        now: Instant,
        intents: &mut Vec<Intent>,
    ) {
        for action in actions {
            match action {
                Action::Forward(input) => self.forward_or_block(input, now, intents),
                Action::Command(PrefixCommand::Detach) => {
                    intents.push(Intent::Detach);
                    return;
                }
                Action::Command(PrefixCommand::ScrollMode) => self.enter_scroll_mode(intents),
                Action::Command(PrefixCommand::SessionPicker) => {
                    intents.push(Intent::OpenPicker);
                    return;
                }
                Action::Command(PrefixCommand::StatusLine) => {
                    self.toggle_status_line(now, intents);
                }
                Action::Command(PrefixCommand::ViewerMode) => {
                    self.toggle_viewer_mode(now, intents);
                }
                Action::Command(PrefixCommand::TeamMember(number)) => {
                    if let Some(id) = self.member_by_number(number) {
                        self.select_member(id, intents);
                    }
                }
                Action::Command(PrefixCommand::TeamNext) => {
                    if let Some(id) = self.next_member(false) {
                        self.select_member(id, intents);
                    }
                }
                Action::Command(PrefixCommand::TeamPrev) => {
                    if let Some(id) = self.next_member(true) {
                        self.select_member(id, intents);
                    }
                }
                Action::Command(PrefixCommand::Release) => {
                    if !self.viewer.is_viewer() {
                        intents.push(Intent::Release);
                    }
                }
            }
        }
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

    fn member_selectable(&self, selected: &str) -> bool {
        self.geometry.granted_top > 0
            && selected != self.session
            && self
                .status
                .as_ref()
                .and_then(|status| status.members.iter().find(|member| member.id == selected))
                .is_some_and(|member| !member.exited)
    }

    fn select_member(&mut self, selected: String, intents: &mut Vec<Intent>) {
        if !self.member_selectable(&selected) {
            return;
        }
        self.mouse_press = None;
        intents.push(Intent::Switch { to: selected });
    }

    fn toggle_status_line(&mut self, now: Instant, intents: &mut Vec<Intent>) {
        let visible = !self.geometry.viewport.status_visible;
        let cols = self.geometry.viewport.cols;
        let rows = self.geometry.viewport.rows;
        if !visible {
            if self.geometry.granted_top > 0 {
                let clear = status::render_team_line(&self.session, "", &[], cols);
                intents.push(Intent::Output(clear.bytes));
                if self.geometry.granted_top == 2 {
                    intents.push(Intent::Output(status::render_team_rule(cols, false)));
                }
                self.team_regions.clear();
            }
            intents.push(Intent::Output(status::clear_status(cols, rows)));
        }
        self.geometry.viewport.status_visible = visible;
        let desired_top = self.geometry.desired_top();
        if desired_top != self.geometry.granted_top {
            self.mouse_press = None;
            intents.push(Intent::Switch {
                to: self.session.clone(),
            });
        } else if self.viewer.is_viewer() {
            intents.push(Intent::Resize {
                cols,
                rows: self.geometry.pty_rows(),
            });
            self.draw_status(intents);
            self.draw_viewer_decorations(now, intents);
        } else {
            intents.push(Intent::Resize {
                cols,
                rows: self.geometry.pty_rows(),
            });
        }
    }

    fn enter_scroll_mode(&mut self, intents: &mut Vec<Intent>) {
        self.mode = Mode::Scroll(ScrollParser::default());
        intents.push(Intent::Redraw);
    }

    fn handle_mouse_button(&mut self, button: u16, intents: &mut Vec<Intent>) {
        if matches!(self.mode, Mode::Picker(_)) {
            return;
        }
        match button {
            64 => self.wheel_scroll_up(intents),
            65 if matches!(self.mode, Mode::Scroll(_)) => Self::wheel_scroll_down(intents),
            _ => {}
        }
    }

    fn wheel_scroll_up(&mut self, intents: &mut Vec<Intent>) {
        if !matches!(self.mode, Mode::Scroll(_)) {
            self.enter_scroll_mode(intents);
        }
        // shortcut: the wheel uses a fixed three-line step; make it configurable only if asked.
        for _ in 0..3 {
            intents.push(Intent::Scroll(Scroll::LineUp));
        }
    }

    fn wheel_scroll_down(intents: &mut Vec<Intent>) {
        // shortcut: the wheel uses a fixed three-line step; make it configurable only if asked.
        for _ in 0..3 {
            intents.push(Intent::Scroll(Scroll::LineDown));
        }
    }

    fn handle_mouse_report(&mut self, report: MouseReport<'_>, intents: &mut Vec<Intent>) {
        match mouse::shift_row(&report, self.geometry.granted_top) {
            Shifted::Claimed(claimed) => self.handle_claimed_mouse(claimed, intents),
            Shifted::Grid(bytes) => self.handle_grid_mouse(report, bytes, intents),
        }
    }

    fn handle_claimed_mouse(&mut self, claimed: MouseReport<'_>, intents: &mut Vec<Intent>) {
        if claimed.release {
            self.claimed_release(claimed, intents);
        } else {
            self.claimed_press(&claimed);
        }
    }

    fn claimed_release(&mut self, claimed: MouseReport<'_>, intents: &mut Vec<Intent>) {
        let session_mouse = self.session_mouse && !self.viewer.is_viewer();
        match self.mouse_press.take() {
            Some(MousePress::Team) if claimed.row == 1 => {
                self.team_click(claimed.column, intents);
            }
            Some(MousePress::Grid) if session_mouse => {
                intents.push(Intent::Forward(mouse::with_row(&claimed, 1)));
            }
            _ => {}
        }
    }

    fn claimed_press(&mut self, claimed: &MouseReport<'_>) {
        if claimed.button == 0 {
            self.mouse_press = Some(match claimed.row {
                1 => MousePress::Team,
                2 => MousePress::Rule,
                _ => return,
            });
        }
    }

    fn handle_grid_mouse(
        &mut self,
        report: MouseReport<'_>,
        bytes: Vec<u8>,
        intents: &mut Vec<Intent>,
    ) {
        let session_mouse = self.session_mouse && !self.viewer.is_viewer();
        if report.release {
            match self.mouse_press.take() {
                Some(MousePress::Team) => {}
                Some(MousePress::Grid) | None if session_mouse => {
                    intents.push(Intent::Forward(bytes));
                }
                _ => {}
            }
            return;
        }
        if report.button == 0 {
            self.mouse_press = Some(MousePress::Grid);
        }
        if session_mouse {
            intents.push(Intent::Forward(bytes));
        } else {
            self.handle_mouse_button(report.button, intents);
        }
    }

    fn team_click(&mut self, column: u16, intents: &mut Vec<Intent>) {
        let target = self
            .team_regions
            .iter()
            .find(|region| region.first_col <= column && column <= region.last_col)
            .map(|region| region.target.clone());
        match target {
            Some(status::TeamTarget::Picker) => intents.push(Intent::OpenPicker),
            Some(status::TeamTarget::Session(id)) => self.select_member(id, intents),
            None => {}
        }
    }

    fn scan_session_mouse(&mut self, bytes: &[u8]) -> bool {
        let mut found = false;
        let mut offset = 0;
        while offset + 3 < bytes.len() {
            if let Some(enabled) = mouse_mode_toggle(bytes, offset) {
                found = true;
                self.session_mouse = enabled;
            }
            offset += 1;
        }
        found
    }

    fn handle_picker_input(&mut self, bytes: &[u8], escape_alone: bool, intents: &mut Vec<Intent>) {
        self.clear_picker_footer(intents);
        let actions = match &mut self.mode {
            Mode::Picker(screen) => screen.feed(bytes, escape_alone),
            _ => return,
        };
        for action in actions {
            if !self.apply_picker_action(action, intents) {
                return;
            }
        }
    }

    fn clear_picker_footer(&mut self, intents: &mut Vec<Intent>) {
        let Mode::Picker(screen) = &mut self.mode else {
            return;
        };
        if screen.footer.take().is_some() {
            intents.push(Intent::Output(screen.render()));
        }
    }

    fn apply_picker_action(&mut self, action: PickerAction, intents: &mut Vec<Intent>) -> bool {
        let outcome = {
            let Mode::Picker(screen) = &mut self.mode else {
                return false;
            };
            let outcome = screen.apply(action);
            screen.parser.set_filter_mode(screen.view.filter_mode());
            outcome
        };
        match outcome {
            None => {
                if let Mode::Picker(screen) = &self.mode {
                    intents.push(Intent::Output(screen.render()));
                }
                true
            }
            Some(Outcome::Cancel) => {
                intents.push(Intent::Redraw);
                self.mode = Mode::Live;
                false
            }
            Some(Outcome::Attach(selected)) => {
                if selected == self.session {
                    intents.push(Intent::Redraw);
                    self.mode = Mode::Live;
                    return false;
                }
                self.mouse_press = None;
                intents.push(Intent::Switch { to: selected });
                false
            }
        }
    }
}

fn mouse_mode_toggle(bytes: &[u8], offset: usize) -> Option<bool> {
    if !bytes[offset..].starts_with(b"\x1b[?") {
        return None;
    }
    let first = offset + 3;
    let mut end = first;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }
    if end == first || !matches!(bytes.get(end), Some(b'h' | b'l')) {
        return None;
    }
    let mode = parse_decimal(&bytes[first..end]);
    matches!(mode, 1000 | 1002 | 1003).then(|| bytes[end] == b'h')
}
