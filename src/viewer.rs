use std::time::{Duration, Instant};

const OVERLAY_DURATION: Duration = Duration::from_secs(2);

pub struct Viewer {
    viewer: bool,
    last_own_input: Instant,
    overlay_until: Option<Instant>,
}

impl Viewer {
    pub fn new(now: Instant, viewer: bool) -> Self {
        Self {
            viewer,
            last_own_input: now,
            overlay_until: None,
        }
    }

    pub fn is_viewer(&self) -> bool {
        self.viewer
    }

    pub fn own_input(&mut self, now: Instant) {
        if !self.viewer {
            self.last_own_input = now;
        }
    }

    pub fn toggle(&mut self, now: Instant) -> bool {
        self.viewer = !self.viewer;
        if !self.viewer {
            self.last_own_input = now;
            self.overlay_until = None;
        }
        self.viewer
    }

    pub fn idle_expired(&mut self, now: Instant, after: Option<Duration>) -> bool {
        if self.viewer {
            return false;
        }
        let Some(after) = after else {
            return false;
        };
        if now.saturating_duration_since(self.last_own_input) >= after {
            self.viewer = true;
            return true;
        }
        false
    }

    pub fn blocked(&mut self, now: Instant) {
        let until = now + OVERLAY_DURATION;
        self.overlay_until = Some(
            self.overlay_until
                .map_or(until, |current| current.max(until)),
        );
    }

    pub fn overlay_active(&self, now: Instant) -> bool {
        self.overlay_until.is_some_and(|until| now < until)
    }

    pub fn take_overlay_end(&mut self, now: Instant) -> bool {
        if self.overlay_until.is_some_and(|until| now >= until) {
            self.overlay_until = None;
            true
        } else {
            false
        }
    }
}

pub fn prefix_name(prefix: u8) -> String {
    match prefix {
        0 => "Ctrl-Space".to_owned(),
        1..=26 => format!("Ctrl-{}", char::from(b'A' + prefix - 1)),
        0x1b => "Ctrl-[".to_owned(),
        0x1c => "Ctrl-\\".to_owned(),
        0x1d => "Ctrl-]".to_owned(),
        0x1e => "Ctrl-^".to_owned(),
        0x1f => "Ctrl-_".to_owned(),
        other => format!("0x{other:02x}"),
    }
}

pub fn overlay(cols: u16, pty_rows: u16, top_rows: u16, prefix: u8) -> Vec<u8> {
    let first = "VIEW MODE";
    let second = format!("typing is off, press {} v to type", prefix_name(prefix));
    let text_width = second.len() as u16;
    let inner_width = text_width.saturating_add(2);
    let width = text_width.saturating_add(4);
    if cols < width || pty_rows < 4 {
        return Vec::new();
    }

    let col = (cols - width) / 2 + 1;
    let first_row = top_rows + (pty_rows - 4) / 2 + 1;
    let mut bytes = b"\x1b7".to_vec();
    for offset in 0..4 {
        let row = first_row + offset;
        bytes.extend_from_slice(format!("\x1b[{row};{col}H").as_bytes());
        if offset == 0 {
            bytes.extend_from_slice(b"\x1b[0m");
        }
        match offset {
            0 => {
                bytes.extend_from_slice("┌".as_bytes());
                for _ in 0..inner_width {
                    bytes.extend_from_slice("─".as_bytes());
                }
                bytes.extend_from_slice("┐".as_bytes());
            }
            1 => {
                bytes.extend_from_slice("│ ".as_bytes());
                bytes.extend_from_slice(first.as_bytes());
                bytes.extend(std::iter::repeat_n(b' ', text_width as usize - first.len()));
                bytes.extend_from_slice(" │".as_bytes());
            }
            2 => {
                bytes.extend_from_slice("│ ".as_bytes());
                bytes.extend_from_slice(second.as_bytes());
                bytes.extend_from_slice(" │".as_bytes());
            }
            _ => {
                bytes.extend_from_slice("└".as_bytes());
                for _ in 0..inner_width {
                    bytes.extend_from_slice("─".as_bytes());
                }
                bytes.extend_from_slice("┘".as_bytes());
            }
        }
    }
    bytes.extend_from_slice(b"\x1b8");
    bytes
}

pub fn marker(cols: u16, rows: u16) -> Vec<u8> {
    if rows < 4 || cols < 12 {
        return Vec::new();
    }
    format!(
        "\x1b7\x1b[{};{}H\x1b[1;30;43m VIEW \x1b[0m\x1b8",
        rows - 1,
        cols - 6
    )
    .into_bytes()
}
