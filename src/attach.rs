//! The interactive attach screen.

mod event_loop;
mod input;
pub mod model;
mod picker;
mod state;

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::client::{Attachment, Client};
use crate::lookup::session_in_team;
use crate::prefix::PrefixMachine;
use crate::status;
use crate::terminal::{Output, OutputHandle, TerminalGuard, write_stderr};
use crate::viewer::Viewer;
use crate::wire::{ClientFrame, StatusInfo};

use self::event_loop::run_attachment_loop;
use self::model::{Geometry, Mode, Viewport};

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
    let Opened {
        attachment,
        in_team,
        ..
    } = open_attachment(
        client,
        home,
        &session,
        force,
        false,
        Viewport {
            cols,
            rows,
            status_visible: true,
        },
    )
    .await?;
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

struct Opened {
    attachment: Attachment,
    top_rows: u16,
    in_team: bool,
}

async fn open_attachment(
    client: Client,
    home: &Path,
    session: &str,
    force: bool,
    viewer: bool,
    viewport: Viewport,
) -> anyhow::Result<Opened> {
    let in_team = session_in_team(home, session).await?;
    let requested_top = viewport.desired_top(in_team);
    let mut attachment = if viewer {
        client
            .attach_viewer(
                session,
                force,
                viewport.cols,
                viewport.pty_rows(requested_top),
                requested_top,
            )
            .await?
    } else {
        client
            .attach_status(
                session,
                force,
                viewport.cols,
                viewport.pty_rows(requested_top),
                requested_top,
            )
            .await?
    };
    let top_rows = attachment.top_rows();
    // shortcut: top_rows is fixed at attach, switch and toggle; resize keeps the negotiated origin until another attach.
    if requested_top > 0 && top_rows == 0 {
        attachment
            .send(ClientFrame::Resize {
                cols: viewport.cols,
                rows: viewport.pty_rows(0),
            })
            .await?;
    }
    Ok(Opened {
        attachment,
        top_rows,
        in_team,
    })
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
    geometry: Geometry,
    team_regions: Vec<status::TeamRegion>,
    mouse_press: Option<MousePress>,
    prefix_machine: PrefixMachine,
    prefix_byte: u8,
    after_prefix: bool,
    viewer: Viewer,
    viewer_after: Option<Duration>,
    status: Option<StatusInfo>,
    session_mouse: bool,
    mode: Mode,
    picker_control: Option<Client>,
    output: OutputHandle,
}
