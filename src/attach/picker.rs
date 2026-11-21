//! The session picker: `PickerScreen`, rendering, entries and row layout.

use crate::picker::{
    EntryState, Outcome, PickerAction, PickerEntry, PickerParser, PickerView, scroll_into_view,
};
use crate::table::{column_widths, format_row, session_value_rows};
use crate::wire::{Activity, SessionSummary};

pub(super) struct PickerScreen {
    pub(super) sessions: Vec<SessionSummary>,
    pub(super) current: String,
    pub(super) view: PickerView,
    pub(super) cols: u16,
    pub(super) rows: u16,
    pub(super) offset: usize,
    pub(super) parser: PickerParser,
    pub(super) footer: Option<String>,
}

const PICKER_HEADERS: [&str; 11] = [
    "ID", "NAME", "HARNESS", "STATE", "ACTIVITY", "ATTACHED", "PENDING", "HELD", "QUOTA", "SIZE",
    "CWD",
];
fn picker_value_rows(sessions: &[SessionSummary]) -> Vec<[String; 11]> {
    session_value_rows(sessions)
        .into_iter()
        .zip(sessions)
        .map(|(row, session)| {
            let [
                id,
                name,
                harness,
                state,
                activity,
                attached,
                pending,
                held,
                quota,
                size,
            ] = row;
            [
                id,
                name,
                harness,
                state,
                activity,
                attached,
                pending,
                held,
                quota,
                size,
                session.cwd.clone().unwrap_or_else(|| "-".to_owned()),
            ]
        })
        .collect()
}
impl PickerScreen {
    pub(super) fn new(sessions: Vec<SessionSummary>, current: &str, cols: u16, rows: u16) -> Self {
        let view = PickerView::new(picker_entries(&sessions), current);
        let mut screen = Self {
            sessions,
            current: current.to_owned(),
            view,
            cols,
            rows,
            offset: 0,
            parser: PickerParser::default(),
            footer: None,
        };
        screen.recompute_offset();
        screen
    }

    pub(super) fn feed(&mut self, bytes: &[u8], escape_alone: bool) -> Vec<PickerAction> {
        self.parser.feed(bytes, escape_alone)
    }

    fn height(&self) -> usize {
        usize::from(self.rows.saturating_sub(3))
    }

    pub(super) fn recompute_offset(&mut self) {
        self.offset = scroll_into_view(
            self.offset,
            self.view.selected_line().unwrap_or(0),
            self.height(),
            self.view.lines().len(),
        );
    }

    pub(super) fn apply(&mut self, action: PickerAction) -> Option<Outcome> {
        let outcome = self.view.apply(action, self.height());
        self.recompute_offset();
        outcome
    }

    pub(super) fn replace_sessions(&mut self, sessions: Vec<SessionSummary>) {
        self.sessions = sessions;
        self.view.replace(picker_entries(&self.sessions));
        self.recompute_offset();
    }

    pub(super) fn render(&self) -> Vec<u8> {
        let mut text = String::from("\x1b[2J\x1b[?25l");
        picker_line(
            &mut text,
            1,
            "a2amx sessions: Enter attach/fold, Esc or q cancel, / filter, Space fold, j/k move",
            self.cols,
        );
        let indent = if self.sessions.iter().any(|session| session.team.is_some()) {
            2
        } else {
            0
        };
        let (header, rows) = session_rows(&self.sessions, &self.current, self.cols, indent);
        picker_line(&mut text, 2, &header, self.cols);
        let lines = self.view.lines();
        let height = self.height();
        let start = self.offset.min(lines.len());
        let end = start.saturating_add(height).min(lines.len());
        let selected = self.view.selected_line();
        for (index, line) in lines[start..end].iter().enumerate() {
            let line_index = start + index;
            let marker = if selected == Some(line_index) {
                '>'
            } else {
                ' '
            };
            let rendered = match line {
                crate::picker::Line::Header { text, .. } => format!("{marker}{text}"),
                crate::picker::Line::Session { id } => {
                    let row = rows
                        .iter()
                        .find(|(row_id, _)| row_id == id)
                        .map_or("", |(_, row)| row.as_str());
                    format!("{marker}{row}")
                }
            };
            picker_line(&mut text, (index + 3) as u16, &rendered, self.cols);
        }
        let footer = self.footer.clone().unwrap_or_else(|| {
            let position = || format!("{}/{}", selected.map_or(0, |line| line + 1), lines.len());
            if self.view.filter_mode() {
                if lines.len() > height {
                    format!("/{}  {}", self.view.query(), position())
                } else {
                    format!("/{}", self.view.query())
                }
            } else if lines.len() > height {
                position()
            } else {
                String::new()
            }
        });
        picker_line(&mut text, self.rows.max(1), &footer, self.cols);
        text.into_bytes()
    }
}

fn picker_line(text: &mut String, row: u16, line: &str, cols: u16) {
    // shortcut: picker clipping counts Unicode scalar values, not terminal cell widths.
    let line = line.chars().take(usize::from(cols)).collect::<String>();
    text.push_str(&format!("\x1b[{row};1H\x1b[2K{line}"));
}
fn picker_entries(sessions: &[SessionSummary]) -> Vec<PickerEntry> {
    sessions
        .iter()
        .map(|session| {
            let state = if session.exit_code.is_some() {
                EntryState::Exited
            } else {
                match session.activity {
                    Some(Activity::Working) => EntryState::Working,
                    Some(Activity::Busy) => EntryState::Busy,
                    Some(Activity::Idle) => EntryState::Idle,
                    None => EntryState::Unknown,
                }
            };
            PickerEntry {
                id: session.id.clone(),
                name: session.name.clone().unwrap_or_default(),
                team: session.team.as_ref().map(|team| team.name.clone()),
                private: session.team.as_ref().is_some_and(|team| team.private),
                state,
                held: session.held,
            }
        })
        .collect()
}
// shortcut: picker widths use byte/scalar counts rather than terminal cell widths.
fn session_rows(
    sessions: &[SessionSummary],
    current: &str,
    cols: u16,
    indent: usize,
) -> (String, Vec<(String, String)>) {
    let rows: Vec<Vec<String>> = picker_value_rows(sessions)
        .into_iter()
        .map(|row| row.into_iter().collect())
        .collect();
    let headers = PICKER_HEADERS.to_vec();
    let widths = column_widths(&headers, &rows);
    let cwd = headers.len() - 1;
    let fixed_prefix_width = widths[..cwd].iter().sum::<usize>() + 2 * cwd;
    let header_has_cwd = usize::from(cols) >= 1 + indent + fixed_prefix_width + headers[cwd].len();
    let mut header = String::from(" ");
    header.push_str(&" ".repeat(indent));
    if header_has_cwd {
        header.push_str(&format_row(&headers, &widths));
    } else {
        header.push_str(&format_row(&headers[..cwd], &widths[..cwd]));
    }
    let lines = rows
        .into_iter()
        .zip(sessions)
        .map(|(row, session)| {
            let suffix = if session.id == current {
                " (current)"
            } else {
                ""
            };
            let mut line = " ".repeat(indent);
            let available =
                usize::from(cols).saturating_sub(1 + indent + fixed_prefix_width + suffix.len());
            if available >= 4 {
                let mut row = row;
                row[cwd] = shorten_left(&row[cwd], available);
                line.push_str(&format_row(&row, &widths));
            } else {
                line.push_str(&format_row(&row[..cwd], &widths[..cwd]));
            }
            line.push_str(suffix);
            (session.id.clone(), line)
        })
        .collect();
    (header, lines)
}

fn shorten_left(value: &str, width: usize) -> String {
    if value.chars().count() <= width {
        return value.to_owned();
    }
    if width <= 1 {
        return "…".chars().take(width).collect();
    }
    let tail = value
        .chars()
        .rev()
        .take(width - 1)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    format!("…{tail}")
}
