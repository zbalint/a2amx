//! Reads remaining quota from harness status lines and OMP conversation errors.

use crate::emulator::Screen;
use crate::harness::Harness;
use crate::wire::QuotaInfo;

/// Status lines sit on the last rows; text higher up is conversation.
const SCANNED_ROWS: usize = 3;
// shortcut: a long multi-line draft in the editor can push the error out of this region; upgrade by
// anchoring on the editor frame if that is seen.
const OMP_SCANNED_ROWS: usize = 12;
const OMP_LIMIT_CODE: &str = "code=usage_limit_reached";

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
pub fn read(screen: &Screen, harness: Harness) -> Option<QuotaInfo> {
    match harness {
        Harness::Claude | Harness::Codex => read_windows(screen),
        Harness::Omp => read_omp_limit(screen),
        Harness::Generic => None,
    }
}

/// Formats quota information as the compact list/status cell.
pub fn cell(quota: Option<QuotaInfo>) -> String {
    let Some(quota) = quota else {
        return "-".to_owned();
    };
    let windows = [("5h", quota.five_hour), ("wk", quota.weekly)];
    let cell = windows
        .into_iter()
        .filter_map(|(label, percent)| percent.map(|percent| format!("{label} {percent}%")))
        .collect::<Vec<_>>()
        .join(" ");
    if cell.is_empty() && quota.limit_reached {
        "limit".to_owned()
    } else {
        cell
    }
}

fn read_windows(screen: &Screen) -> Option<QuotaInfo> {
    let rows = usize::from(screen.size.rows);
    let mut info = QuotaInfo::default();
    for row in (rows.saturating_sub(SCANNED_ROWS)..rows).rev() {
        let text = row_to_text(screen, row)?;
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

fn read_omp_limit(screen: &Screen) -> Option<QuotaInfo> {
    let rows = usize::from(screen.size.rows);
    for row in (rows.saturating_sub(OMP_SCANNED_ROWS)..rows).rev() {
        let current: String = row_to_text(screen, row)?.into_iter().collect();
        if !current.trim().starts_with("Error: ") {
            continue;
        }
        let below: String = if row + 1 < rows {
            row_to_text(screen, row + 1)?.into_iter().collect()
        } else {
            String::new()
        };
        if current.contains(OMP_LIMIT_CODE) || below.contains(OMP_LIMIT_CODE) {
            return Some(QuotaInfo {
                limit_reached: true,
                ..Default::default()
            });
        }
    }
    None
}

fn row_to_text(screen: &Screen, row: usize) -> Option<Vec<char>> {
    let cols = usize::from(screen.size.cols);
    let mut text: Vec<char> = screen
        .cells
        .get(row * cols..(row + 1) * cols)?
        .iter()
        .map(|cell| if cell.ch == '\u{a0}' { ' ' } else { cell.ch })
        .collect();
    while text.last() == Some(&' ') {
        text.pop();
    }
    Some(text)
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
