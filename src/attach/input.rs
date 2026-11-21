//! Parsing terminal input: scroll keys, mouse reports, focus reports.

use crate::emulator::Scroll;
use crate::mouse::{self, MouseReport};

pub(super) fn parse_decimal(bytes: &[u8]) -> u16 {
    bytes
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .fold(0u16, |value, byte| {
            value
                .saturating_mul(10)
                .saturating_add(u16::from(byte - b'0'))
        })
}
pub(super) fn is_focus_report(bytes: &[u8]) -> bool {
    matches!(bytes, b"\x1b[I" | b"\x1b[O")
}

// shortcut: a report split across input chunks is typed input; reassemble only if it proves noisy; this scan covers SGR and X10.
pub(super) fn find_mouse_report(bytes: &[u8], start: usize) -> Option<(usize, MouseReport<'_>)> {
    let mut offset = start;
    while offset < bytes.len() {
        if bytes[offset] == 0x1b
            && let Some(report) = mouse::parse_report(bytes, offset)
        {
            return Some((offset, report));
        }
        offset += 1;
    }
    None
}
#[derive(Default)]
pub(super) struct ScrollParser {
    pending: Vec<u8>,
}

pub(super) enum ScrollKey {
    Scroll(Scroll),
    Quit,
}

impl ScrollParser {
    pub(super) fn feed(&mut self, bytes: &[u8]) -> Vec<ScrollKey> {
        let mut keys = Vec::new();
        for byte in bytes {
            if self.pending.is_empty() && *byte == b'q' {
                keys.push(ScrollKey::Quit);
                continue;
            }
            self.pending.push(*byte);
            if let Some(key) = parse_scroll_sequence(&self.pending) {
                self.pending.clear();
                keys.push(ScrollKey::Scroll(key));
            } else if self.pending.len() > 5 || self.pending.first() != Some(&0x1b) {
                self.pending.clear();
            }
        }
        keys
    }
}

fn parse_scroll_sequence(bytes: &[u8]) -> Option<Scroll> {
    match bytes {
        b"\x1b[5~" => Some(Scroll::PageUp),
        b"\x1b[6~" => Some(Scroll::PageDown),
        b"\x1b[A" | b"\x1bOA" => Some(Scroll::LineUp),
        b"\x1b[B" | b"\x1bOB" => Some(Scroll::LineDown),
        b"\x1b[H" | b"\x1b[1~" | b"\x1bOH" => Some(Scroll::Top),
        b"\x1b[F" | b"\x1b[4~" | b"\x1bOF" => Some(Scroll::Bottom),
        _ => None,
    }
}

pub(super) fn scroll_status(cols: u16, rows: u16) -> Vec<u8> {
    let status = "[scroll: q to exit]";
    let width = status.len() as u16;
    let col = cols.saturating_sub(width).saturating_add(1).max(1);
    format!("\x1b[{rows};{col}H\x1b[7m{status}\x1b[0m\x1b[?25l").into_bytes()
}
