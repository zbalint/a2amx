pub(super) const BAR_FOREGROUND: &str = "\x1b[38;5;252m";
pub(super) const BAR_BACKGROUND: &str = "\x1b[38;5;252;48;5;236m";
pub(super) const NAME_OPEN: &str = "\x1b[1m";
pub(super) const NAME_CLOSE: &str = "\x1b[22m";
const SEPARATOR_OPEN: &str = "\x1b[38;5;240m";

#[derive(Clone)]
pub(super) struct Span {
    open: &'static str,
    text: String,
    close: &'static str,
}

impl Span {
    pub(super) fn plain(text: impl Into<String>) -> Self {
        Self {
            open: "",
            text: text.into(),
            close: "",
        }
    }

    pub(super) fn styled(open: &'static str, text: impl Into<String>, close: &'static str) -> Self {
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

pub(super) fn render_separator(row: u16, width: usize) -> Vec<u8> {
    let mut bytes = format!("\x1b[{row};1H\x1b[0m\x1b[2K{SEPARATOR_OPEN}").into_bytes();
    for _ in 0..width {
        bytes.extend_from_slice("─".as_bytes());
    }
    bytes.extend_from_slice(b"\x1b[0m");
    bytes
}

pub(super) fn sanitize(text: &str) -> String {
    text.chars()
        .map(|ch| if ch.is_control() { '?' } else { ch })
        .collect()
}

pub(super) fn char_len(text: &str) -> usize {
    text.chars().count()
}

pub(super) fn spans_len(spans: &[Span]) -> usize {
    spans.iter().map(Span::len).sum()
}

pub(super) fn detail_prefix() -> [Span; 2] {
    [
        Span::styled(SEPARATOR_OPEN, "│", BAR_FOREGROUND),
        Span::plain(" "),
    ]
}

pub(super) fn render_spans(spans: &[Span], limit: usize) -> String {
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

pub(super) fn render_span(span: &Span, limit: usize) -> String {
    if limit == 0 {
        return String::new();
    }
    let text: String = span.text.chars().take(limit).collect();
    if text.is_empty() {
        return String::new();
    }
    format!("{}{text}{}", span.open, span.close)
}
