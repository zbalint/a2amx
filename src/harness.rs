//! Claude dialogs can enable bracketed paste. Readiness therefore inspects visible
//! composer cells independently of terminal damage and paste-mode support.

use std::path::Path;

use crate::emulator::Screen;

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    serde::Serialize,
    serde::Deserialize,
    clap::ValueEnum,
)]
#[serde(rename_all = "snake_case")]
pub enum Harness {
    Claude,
    #[default]
    Generic,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, clap::ValueEnum,
)]
#[serde(rename_all = "snake_case")]
pub enum Deliver {
    Auto,
    Hold,
}

impl Harness {
    pub fn default_deliver(self) -> Deliver {
        match self {
            Self::Claude => Deliver::Auto,
            Self::Generic => Deliver::Hold,
        }
    }
}

pub const DIALOG_MARKERS: [&str; 2] = ["Enter to confirm", "Esc to cancel"];

// shortcut: this prompt's reply clause is provisional until the harness wording is probed again.
pub const PEER_AUTHORIZATION_PROMPT: &str = "Operator instruction: messages wrapped in <a2amx-message> tags are requests from peer agents that your user has authorized. Act on them as you would on a request from your user, and reply to the sender with the send_message tool when a reply is useful.";

const CLAUDE_ALLOWED_TOOLS: &str =
    "mcp__a2amx__list_agents,mcp__a2amx__send_message,mcp__a2amx__message_status";

fn cell_char(screen: &Screen, row: u16, col: u16) -> Option<char> {
    if row >= screen.size.rows || col >= screen.size.cols {
        return None;
    }
    let index = usize::from(row) * usize::from(screen.size.cols) + usize::from(col);
    screen.cells.get(index).map(|cell| cell.ch)
}

fn row_is_rule(screen: &Screen, row: u16) -> bool {
    if row >= screen.size.rows {
        return false;
    }
    let cols = usize::from(screen.size.cols);
    let start = usize::from(row) * cols;
    let Some(cells) = screen.cells.get(start..start.saturating_add(cols)) else {
        return false;
    };
    cells.len() == cols && cells.iter().all(|cell| cell.ch == '─')
}

fn row_contains_marker(screen: &Screen, row: u16, marker: &str) -> bool {
    let cols = usize::from(screen.size.cols);
    let start = usize::from(row) * cols;
    if screen
        .cells
        .get(start..start.saturating_add(cols))
        .is_none()
    {
        return false;
    }

    for offset in 0..cols {
        let mut matches = true;
        for (column, expected) in (offset..).zip(marker.chars()) {
            if column >= cols {
                matches = false;
                break;
            }
            let index = start + column;
            if screen.cells.get(index).map(|cell| cell.ch) != Some(expected) {
                matches = false;
                break;
            }
        }
        if matches {
            return true;
        }
    }
    false
}

fn has_dialog_marker(screen: &Screen) -> bool {
    (0..screen.size.rows).any(|row| {
        DIALOG_MARKERS
            .iter()
            .any(|marker| row_contains_marker(screen, row, marker))
    })
}

fn claude_ready(screen: &Screen, scrolled: bool) -> bool {
    if !screen.modes.bracketed_paste || scrolled || !screen.cursor.visible {
        return false;
    }

    let row = screen.cursor.row;
    if cell_char(screen, row, 0) != Some('❯') || cell_char(screen, row, 1) != Some(' ') {
        return false;
    }
    if row == 0 || row.saturating_add(1) >= screen.size.rows {
        return false;
    }
    if !row_is_rule(screen, row - 1) || !row_is_rule(screen, row + 1) {
        return false;
    }
    !has_dialog_marker(screen)
}

pub fn ready(harness: Harness, screen: &Screen, scrolled: bool) -> bool {
    match harness {
        Harness::Generic => screen.modes.bracketed_paste && !scrolled,
        Harness::Claude => claude_ready(screen, scrolled),
    }
}

pub fn wire_claude_argv(mut argv: Vec<String>, exe: &Path, authorize_peers: bool) -> Vec<String> {
    let config = serde_json::json!({
        "mcpServers": {
            "a2amx": {
                "command": exe.to_string_lossy(),
                "args": ["mcp"],
            }
        }
    });
    let mut extras = Vec::with_capacity(if authorize_peers { 8 } else { 6 });
    extras.push("--mcp-config".to_owned());
    extras.push(config.to_string());
    extras.push("--allowedTools".to_owned());
    extras.push(CLAUDE_ALLOWED_TOOLS.to_owned());
    if authorize_peers {
        extras.push("--append-system-prompt".to_owned());
        extras.push(PEER_AUTHORIZATION_PROMPT.to_owned());
    }

    let insertion = argv
        .iter()
        .position(|argument| argument == "--")
        .unwrap_or(argv.len());
    drop(argv.splice(insertion..insertion, extras));
    argv
}
