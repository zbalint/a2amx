//! The interactive attach screen.

mod event_loop;
mod input;
mod picker;
mod state;

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::client::{Attachment, Client};
use crate::lookup::session_in_team;
use crate::picker::{PickerParser, PickerView};
use crate::prefix::PrefixMachine;
use crate::status;
use crate::terminal::{Output, OutputHandle, TerminalGuard, write_stderr};
use crate::viewer::Viewer;
use crate::wire::{ClientFrame, SessionSummary, StatusInfo};

use self::event_loop::run_attachment_loop;
use self::input::ScrollParser;

const INPUT_CHUNK: usize = 8 * 1024;
const MOUSE_CAPTURE: &[u8] = b"\x1b[?1000h\x1b[?1006h";
#[derive(Clone, Copy)]
pub struct AttachOptions {
    pub prefix: u8,
    pub viewer_after: Option<Duration>,
}
pub async fn run(
    client: Client,
    home: &Path,
    attach_options: AttachOptions,
    session: String,
    force: bool,
    cols: u16,
    rows: u16,
) -> anyhow::Result<()> {
    let AttachOptions {
        prefix,
        viewer_after,
    } = attach_options;
    let in_team = session_in_team(home, &session).await?;
    let requested_top = team_rows(rows, true, in_team);
    let mut attachment = client
        .attach_status(
            &session,
            force,
            cols,
            pty_rows(rows, true, requested_top),
            requested_top,
        )
        .await?;
    let top_rows = attachment.top_rows();
    // shortcut: top_rows is fixed at attach, switch and toggle; resize keeps the negotiated origin until another attach.
    if requested_top > 0 && top_rows == 0 {
        attachment
            .send(ClientFrame::Resize {
                cols,
                rows: pty_rows(rows, true, 0),
            })
            .await?;
    }
    let output = Output::start()?;
    let mut terminal = TerminalGuard::enter(output.handle()?)?;
    output.send(b"\x1b[?1049h".to_vec())?;
    // shortcut: the outer terminal title is not restored on exit.
    output.send(format!("\x1b]2;a2amx: {session}\x07").into_bytes())?;

    let state = AttachmentState::new(
        attachment,
        AttachmentConfig {
            home: home.to_owned(),
            prefix,
            session,
            cols,
            rows,
            in_team,
            viewer_after,
            output: output.handle()?,
        },
    );
    let loop_result = run_attachment_loop(state).await;
    let restore_result = terminal.restore().await;
    let finish_result = tokio::task::spawn_blocking(move || output.finish()).await?;
    restore_result?;
    finish_result?;
    let outcome = loop_result?;
    write_stderr(format!("{}\n", outcome.message).into_bytes()).await
}

fn pty_rows(rows: u16, visible: bool, top: u16) -> u16 {
    let status_rows = if visible && rows >= 4 {
        2
    } else if visible && rows == 3 {
        1
    } else {
        0
    };
    rows.saturating_sub(status_rows).saturating_sub(top).max(1)
}

/// Reserves two top rows for a visible team line and its rule at six rows or more,
/// one row at exactly five rows, and no rows otherwise.
fn team_rows(rows: u16, visible: bool, in_team: bool) -> u16 {
    if !visible || !in_team {
        return 0;
    }
    if rows >= 6 {
        2
    } else if rows == 5 {
        1
    } else {
        0
    }
}

struct AttachOutcome {
    message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MousePress {
    Team,
    Rule,
    Grid,
}

struct AttachmentConfig {
    home: PathBuf,
    prefix: u8,
    session: String,
    cols: u16,
    rows: u16,
    in_team: bool,
    viewer_after: Option<Duration>,
    output: OutputHandle,
}

struct AttachmentState {
    attachment: Option<Attachment>,
    home: PathBuf,
    session: String,
    cols: u16,
    rows: u16,
    in_team: bool,
    top_rows: u16,
    team_regions: Vec<status::TeamRegion>,
    mouse_press: Option<MousePress>,
    prefix_machine: PrefixMachine,
    prefix_byte: u8,
    after_prefix: bool,
    viewer: Viewer,
    viewer_after: Option<Duration>,
    status_visible: bool,
    status: Option<StatusInfo>,
    session_mouse: bool,
    scroll_mode: bool,
    scroll_parser: ScrollParser,
    picker: Option<PickerState>,
    output: OutputHandle,
}
struct PickerState {
    control: Client,
    sessions: Vec<SessionSummary>,
    current: String,
    view: PickerView,
    cols: u16,
    rows: u16,
    offset: usize,
    parser: PickerParser,
    footer: Option<String>,
}
