//! Pure data types for the attach screen's geometry and input mode.

use super::input::ScrollParser;
use super::picker::PickerScreen;

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
pub(super) enum Mode {
    Live,
    Scroll(ScrollParser),
    Picker(Box<PickerScreen>),
}

impl Mode {
    pub(super) fn is_live(&self) -> bool {
        matches!(self, Mode::Live)
    }
}
