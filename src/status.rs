use crate::wire::StatusInfo;

/// Bytes that draw the status bar on terminal row `rows`, leaving the cursor where it was.
pub fn render(session: &str, info: Option<&StatusInfo>, cols: u16, rows: u16) -> Vec<u8> {
    if rows < 3 || cols < 2 {
        return Vec::new();
    }
    // Leave the last column untouched to avoid pending wrap on the last row.
    let width = usize::from(cols - 1);
    let mut left = format!(" {} ", info.map_or(session, |info| &info.address));
    left = left
        .chars()
        .map(|ch| if ch.is_control() { '?' } else { ch })
        .collect();
    let mut pending = info
        .filter(|info| info.pending > 0)
        .map_or_else(String::new, |info| format!(" {} pending ", info.pending));
    let mut hold = info
        .and_then(|info| info.hold.as_ref())
        .map_or_else(String::new, |reason| format!(" HELD {reason} "));
    hold = hold
        .chars()
        .map(|ch| if ch.is_control() { '?' } else { ch })
        .collect();
    // shortcut: widths are char counts, not terminal cell widths; use unicode-width if wide
    // addresses ever matter.
    if left.chars().count() + pending.chars().count() + hold.chars().count() > width {
        pending.clear();
    }
    if left.chars().count() + hold.chars().count() > width {
        if hold.chars().count() >= width {
            hold = hold.chars().take(width).collect();
            left.clear();
        } else {
            left = left.chars().take(width - hold.chars().count()).collect();
        }
    }
    let padding =
        " ".repeat(width - left.chars().count() - pending.chars().count() - hold.chars().count());
    let alert = if hold.is_empty() {
        ""
    } else {
        "\x1b[0m\x1b[1;37;41m"
    };
    format!("\x1b7\x1b[{rows};1H\x1b[0m\x1b[7m{left}{padding}{pending}{alert}{hold}\x1b[0m\x1b8")
        .into_bytes()
}
