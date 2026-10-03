//! Reads the remaining quota from a Claude or Codex status line on the screen.

use crate::emulator::Screen;
use crate::wire::QuotaInfo;

/// Status lines sit on the last rows; text higher up is conversation.
const SCANNED_ROWS: usize = 3;

#[derive(Clone, Copy, PartialEq)]
enum Window {
    FiveHour,
    Weekly,
}

const LABELS: [(&str, Window); 3] = [
    ("5h", Window::FiveHour),
    ("weekly", Window::Weekly),
    ("7d", Window::Weekly),
];

/// What the harness status line on the screen's last rows says about quota, or `None` when it
/// says nothing.
pub fn read(screen: &Screen) -> Option<QuotaInfo> {
    let cols = usize::from(screen.size.cols);
    let rows = usize::from(screen.size.rows);
    let mut info = QuotaInfo::default();
    for row in (rows.saturating_sub(SCANNED_ROWS)..rows).rev() {
        let mut text: Vec<char> = screen
            .cells
            .get(row * cols..(row + 1) * cols)?
            .iter()
            .map(|cell| if cell.ch == '\u{a0}' { ' ' } else { cell.ch })
            .collect();
        while text.last() == Some(&' ') {
            text.pop();
        }
        for start in 0..text.len() {
            if let Some((window, percent)) = token_at(&text, start) {
                let slot = match window {
                    Window::FiveHour => &mut info.five_hour,
                    Window::Weekly => &mut info.weekly,
                };
                slot.get_or_insert(percent);
            }
        }
    }
    (info != QuotaInfo::default()).then_some(info)
}

/// A `<label> <N>% left` token starting at `start`, if one does.
fn token_at(text: &[char], start: usize) -> Option<(Window, u8)> {
    if start > 0 && text[start - 1].is_alphanumeric() {
        return None;
    }
    LABELS.iter().find_map(|(label, window)| {
        let rest = text[start..].strip_prefix(&label.chars().collect::<Vec<_>>()[..])?;
        let rest = rest.strip_prefix(&[' '])?;
        let digits = rest.iter().take_while(|c| c.is_ascii_digit()).count();
        if !(1..=3).contains(&digits) {
            return None;
        }
        let percent: u8 = rest[..digits]
            .iter()
            .collect::<String>()
            .parse::<u16>()
            .ok()
            .filter(|value| *value <= 100)?
            .try_into()
            .ok()?;
        let rest = rest[digits..].strip_prefix(&['%', ' ', 'l', 'e', 'f', 't'])?;
        match rest.first() {
            Some(c) if c.is_alphanumeric() => None,
            _ => Some((*window, percent)),
        }
    })
}
