use crate::harness::Harness;
use crate::quota;
use crate::wire::{Activity, QuotaInfo, StatusInfo};

const BAR_FOREGROUND: &str = "\x1b[38;5;252m";
const BAR_BACKGROUND: &str = "\x1b[38;5;252;48;5;236m";
const NAME_OPEN: &str = "\x1b[1m";
const NAME_CLOSE: &str = "\x1b[22m";
const SEPARATOR_OPEN: &str = "\x1b[38;5;240m";
const ALERT_OPEN: &str = "\x1b[0m\x1b[1;37;41m";

struct Span {
    open: &'static str,
    text: String,
    close: &'static str,
}

impl Span {
    fn plain(text: impl Into<String>) -> Self {
        Self {
            open: "",
            text: text.into(),
            close: "",
        }
    }

    fn styled(open: &'static str, text: impl Into<String>, close: &'static str) -> Self {
        Self {
            open,
            text: text.into(),
            close,
        }
    }

    fn len(&self) -> usize {
        self.text.chars().count()
    }
}

/// Bytes that draw the status bar on terminal row `rows`, leaving the cursor where it was.
pub fn render(
    session: &str,
    info: Option<&StatusInfo>,
    prefix: u8,
    cols: u16,
    rows: u16,
) -> Vec<u8> {
    if rows < 3 || cols < 2 {
        return Vec::new();
    }
    // Leave the last column untouched to avoid pending wrap on the last row.
    // shortcut: widths are character counts; `●`, `│` and `·` are East Asian Ambiguous width and
    // can misalign on terminals that render ambiguous characters wide.
    let width = usize::from(cols - 1);
    let name = sanitize(info.map_or(session, |info| &info.address));
    let activity = info
        .filter(|info| info.hold.is_none())
        .and_then(|info| info.activity);
    let harness = info.and_then(|info| info.harness);
    let quota = info
        .and_then(|info| info.quota)
        .filter(is_low_quota)
        .map(|quota| (quota::cell(Some(quota)), quota.limit_reached));
    let pending = info
        .filter(|info| info.pending > 0)
        .map(|info| format!(" {} pending ", info.pending));
    let hold_reason = info.and_then(|info| info.hold.as_deref()).map(sanitize);
    let mut show_activity = activity.is_some();
    let mut show_quota = quota.is_some();
    let mut show_harness = harness.is_some();
    let mut show_pending = pending.is_some();
    let mut show_hint = hold_reason
        .as_deref()
        .is_some_and(|reason| hint_allowed(reason, prefix));

    loop {
        let left = left_spans(
            &name,
            show_activity.then_some(activity).flatten(),
            show_quota
                .then(|| quota.as_ref().map(|(text, limit)| (text.as_str(), *limit)))
                .flatten(),
            show_harness.then_some(harness).flatten(),
        );
        let hold = hold_reason
            .as_deref()
            .map(|reason| hold_text(reason, show_hint, prefix));
        let pending_len = if show_pending {
            pending.as_deref().map_or(0, char_len)
        } else {
            0
        };
        let total = spans_len(&left) + pending_len + hold.as_deref().map_or(0, char_len);
        if total <= width {
            break;
        }
        if show_harness {
            show_harness = false;
        } else if show_quota {
            show_quota = false;
        } else if show_activity {
            show_activity = false;
        } else if show_pending {
            show_pending = false;
        } else if show_hint {
            show_hint = false;
        } else {
            break;
        }
    }

    let left = left_spans(
        &name,
        show_activity.then_some(activity).flatten(),
        show_quota
            .then(|| quota.as_ref().map(|(text, limit)| (text.as_str(), *limit)))
            .flatten(),
        show_harness.then_some(harness).flatten(),
    );
    let pending = if show_pending {
        pending.unwrap_or_default()
    } else {
        String::new()
    };
    let hold = hold_reason
        .as_deref()
        .map(|reason| hold_text(reason, show_hint, prefix))
        .unwrap_or_default();
    let left_len = spans_len(&left);
    let hold_len = char_len(&hold);
    let (left_limit, hold_limit) = if left_len + hold_len > width {
        if hold_len >= width {
            (0, width)
        } else {
            (width - hold_len, hold_len)
        }
    } else {
        (left_len, hold_len)
    };
    let left = render_spans(&left, left_limit);
    let hold = render_span(&Span::styled(ALERT_OPEN, hold, ""), hold_limit);
    let padding = " ".repeat(width - left_limit - char_len(&pending) - hold_limit);

    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"\x1b7");
    if rows >= 4 {
        bytes.extend_from_slice(
            format!("\x1b[{};1H\x1b[0m\x1b[2K{SEPARATOR_OPEN}", rows - 1).as_bytes(),
        );
        for _ in 0..width {
            bytes.extend_from_slice("─".as_bytes());
        }
        bytes.extend_from_slice(b"\x1b[0m");
    }
    bytes.extend_from_slice(format!("\x1b[{rows};1H\x1b[0m{BAR_BACKGROUND}").as_bytes());
    bytes.extend_from_slice(left.as_bytes());
    bytes.extend_from_slice(padding.as_bytes());
    bytes.extend_from_slice(pending.as_bytes());
    bytes.extend_from_slice(hold.as_bytes());
    bytes.extend_from_slice(b"\x1b[0m\x1b8");
    bytes
}

fn sanitize(text: &str) -> String {
    text.chars()
        .map(|ch| if ch.is_control() { '?' } else { ch })
        .collect()
}

fn char_len(text: &str) -> usize {
    text.chars().count()
}

fn spans_len(spans: &[Span]) -> usize {
    spans.iter().map(Span::len).sum()
}

fn is_low_quota(quota: &QuotaInfo) -> bool {
    quota.limit_reached
        || quota.five_hour.is_some_and(|percent| percent <= 15)
        || quota.weekly.is_some_and(|percent| percent <= 15)
}

fn hint_allowed(reason: &str, prefix: u8) -> bool {
    matches!(
        reason,
        "human_draft" | "unsubmitted_envelope" | "corrupted_submissions"
    ) && (1..=26).contains(&prefix)
}

fn hold_text(reason: &str, hinted: bool, prefix: u8) -> String {
    if hinted {
        let letter = char::from(b'A' + prefix - 1);
        format!(" HELD {reason} · ^{letter} r ")
    } else {
        format!(" HELD {reason} ")
    }
}

fn left_spans(
    name: &str,
    activity: Option<Activity>,
    quota: Option<(&str, bool)>,
    harness: Option<Harness>,
) -> Vec<Span> {
    let mut spans = vec![
        Span::plain(" "),
        Span::styled(NAME_OPEN, name, NAME_CLOSE),
        Span::plain(" "),
    ];
    if let Some(activity) = activity {
        spans.extend(detail_prefix());
        spans.push(Span::styled(activity_color(activity), "●", BAR_FOREGROUND));
        spans.push(Span::plain(" "));
        spans.push(Span::plain(activity_word(activity)));
        spans.push(Span::plain(" "));
    }
    if let Some((text, limit_reached)) = quota {
        spans.extend(detail_prefix());
        spans.push(Span::styled(
            if limit_reached {
                "\x1b[38;5;203m"
            } else {
                "\x1b[38;5;221m"
            },
            text,
            BAR_FOREGROUND,
        ));
        spans.push(Span::plain(" "));
    }
    if let Some(harness) = harness {
        spans.extend(detail_prefix());
        spans.push(Span::plain(harness.as_str()));
        spans.push(Span::plain(" "));
    }
    spans
}

fn detail_prefix() -> [Span; 2] {
    [
        Span::styled(SEPARATOR_OPEN, "│", BAR_FOREGROUND),
        Span::plain(" "),
    ]
}

fn activity_word(activity: Activity) -> &'static str {
    match activity {
        Activity::Idle => "idle",
        Activity::Working => "working",
        Activity::Busy => "resetting",
    }
}

fn activity_color(activity: Activity) -> &'static str {
    match activity {
        Activity::Idle => "\x1b[38;5;78m",
        Activity::Working => "\x1b[38;5;221m",
        Activity::Busy => "\x1b[38;5;209m",
    }
}

fn render_spans(spans: &[Span], limit: usize) -> String {
    let mut output = String::new();
    let mut remaining = limit;
    for span in spans {
        if remaining == 0 {
            break;
        }
        let text: String = span.text.chars().take(remaining).collect();
        let taken = text.chars().count();
        if taken == 0 {
            continue;
        }
        output.push_str(span.open);
        output.push_str(&text);
        output.push_str(span.close);
        remaining -= taken;
    }
    output
}

fn render_span(span: &Span, limit: usize) -> String {
    if limit == 0 {
        return String::new();
    }
    let text: String = span.text.chars().take(limit).collect();
    if text.is_empty() {
        return String::new();
    }
    format!("{}{text}{}", span.open, span.close)
}
