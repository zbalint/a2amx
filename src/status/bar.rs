use crate::harness::Harness;
use crate::quota;
use crate::wire::{QuotaInfo, RemoteLease, RemoteStatus, StatusInfo};

use super::dot::{DotState, activity_color, activity_word, dot_state};
use super::spans::{
    BAR_BACKGROUND, BAR_FOREGROUND, NAME_CLOSE, NAME_OPEN, Span, char_len, detail_prefix,
    render_separator, render_span, render_spans, sanitize, spans_len,
};

const ALERT_OPEN: &str = "\x1b[0m\x1b[1;37;41m";

/// Bytes that clear the separator and status bar rows, leaving the cursor where it was.
pub fn clear_status(cols: u16, rows: u16) -> Vec<u8> {
    if rows < 3 || cols < 2 {
        return Vec::new();
    }
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"\x1b7");
    if rows >= 4 {
        bytes.extend_from_slice(format!("\x1b[{};1H\x1b[0m\x1b[2K", rows - 1).as_bytes());
    }
    bytes.extend_from_slice(format!("\x1b[{rows};1H\x1b[0m\x1b[2K\x1b8").as_bytes());
    bytes
}

pub struct StatusBar<'a> {
    session: &'a str,
    info: Option<&'a StatusInfo>,
    prefix: u8,
}

impl<'a> StatusBar<'a> {
    pub fn new(session: &'a str, info: Option<&'a StatusInfo>, prefix: u8) -> Self {
        Self {
            session,
            info,
            prefix,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Part {
    Team,
    Harness,
    Role,
    Quota,
    Activity,
    Pending,
    Hint,
    Remote,
    Confined,
}

struct Shown(Vec<Part>);

impl Shown {
    fn all(parts: &Parts) -> Self {
        Self(
            [
                (Part::Team, parts.team.is_some()),
                (Part::Harness, parts.harness.is_some()),
                (Part::Role, parts.role.is_some()),
                (Part::Quota, parts.quota.is_some()),
                (Part::Activity, parts.activity.is_some()),
                (Part::Pending, parts.pending.is_some()),
                (
                    Part::Hint,
                    parts
                        .hold
                        .as_deref()
                        .is_some_and(|reason| hint_allowed(reason, parts.prefix)),
                ),
                (Part::Remote, parts.remote.is_some()),
                (Part::Confined, parts.confine.is_some()),
            ]
            .into_iter()
            .filter_map(|(part, present)| present.then_some(part))
            .collect(),
        )
    }

    fn has(&self, part: Part) -> bool {
        self.0.contains(&part)
    }

    fn pick<T>(&self, part: Part, value: Option<T>) -> Option<T> {
        value.filter(|_| self.has(part))
    }

    fn drop_next(&mut self) -> bool {
        if self.0.is_empty() {
            false
        } else {
            self.0.remove(0);
            true
        }
    }
}

struct Parts {
    name: String,
    confine: Option<bool>,
    activity: Option<DotState>,
    quota: Option<(String, bool)>,
    role: Option<String>,
    team: Option<String>,
    harness: Option<Harness>,
    pending: Option<String>,
    hold: Option<String>,
    remote: Option<Vec<Span>>,
    prefix: u8,
}

impl Parts {
    fn new(bar: &StatusBar<'_>) -> Self {
        let info = bar.info;
        let name = sanitize(info.map_or(bar.session, |info| &info.address));
        let activity = info.filter(|info| info.hold.is_none()).and_then(|info| {
            info.activity
                .map(|activity| dot_state(Some(activity), false, info.starting, false))
        });
        let harness = info.and_then(|info| info.harness);
        let role = info
            .and_then(|info| info.role.as_deref())
            .map(|role| format!("role {}", sanitize(role)));
        let team = info.and_then(|info| info.team.as_ref()).map(|team| {
            let suffix = if team.private { " (private)" } else { "" };
            format!("team {}{suffix}", sanitize(&team.name))
        });
        let quota = info
            .and_then(|info| info.quota)
            .filter(is_low_quota)
            .map(|quota| (quota::cell(Some(quota)), quota.limit_reached));
        let pending = info
            .filter(|info| info.pending > 0)
            .map(|info| format!(" {} pending ", info.pending));
        let hold = info.and_then(|info| info.hold.as_deref()).map(sanitize);
        let remote = info.and_then(|info| info.remote).map(remote_spans);
        let confine = info
            .filter(|info| info.confined)
            .map(|info| info.confine_auto);
        Self {
            name,
            confine,
            activity,
            quota,
            role,
            team,
            harness,
            pending,
            hold,
            remote,
            prefix: bar.prefix,
        }
    }
}

/// Bytes that draw the status bar on terminal row `rows`, leaving the cursor where it was.
pub fn render(bar: &StatusBar<'_>, cols: u16, rows: u16) -> Vec<u8> {
    if rows < 3 || cols < 2 {
        return Vec::new();
    }
    // Leave the last column untouched to avoid pending wrap on the last row.
    // shortcut: widths are character counts; `●`, `│` and `·` are East Asian Ambiguous width and
    // can misalign on terminals that render ambiguous characters wide.
    let width = usize::from(cols - 1);
    let parts = Parts::new(bar);
    let (shown, left) = fit_bar(&parts, width);
    let remote = if shown.has(Part::Remote) {
        parts.remote.unwrap_or_default()
    } else {
        Vec::new()
    };
    let pending = if shown.has(Part::Pending) {
        parts.pending.unwrap_or_default()
    } else {
        String::new()
    };
    let hold = parts
        .hold
        .as_deref()
        .map(|reason| hold_text(reason, shown.has(Part::Hint), parts.prefix))
        .unwrap_or_default();
    let left_len = spans_len(&left);
    let remote_len = spans_len(&remote);
    let hold_len = char_len(&hold);
    let (left_limit, hold_limit) = split_width(left_len, hold_len, width);
    let left = render_spans(&left, left_limit);
    let remote = render_spans(&remote, remote_len);
    let hold = render_span(&Span::styled(ALERT_OPEN, hold, ""), hold_limit);
    let padding = " ".repeat(width - left_limit - remote_len - char_len(&pending) - hold_limit);

    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"\x1b7");
    if rows >= 4 {
        bytes.extend_from_slice(render_separator(rows - 1, width).as_slice());
    }
    bytes.extend_from_slice(format!("\x1b[{rows};1H\x1b[0m{BAR_BACKGROUND}").as_bytes());
    bytes.extend_from_slice(left.as_bytes());
    bytes.extend_from_slice(padding.as_bytes());
    bytes.extend_from_slice(remote.as_bytes());
    bytes.extend_from_slice(pending.as_bytes());
    bytes.extend_from_slice(hold.as_bytes());
    bytes.extend_from_slice(b"\x1b[0m\x1b8");
    bytes
}

fn fit_bar(parts: &Parts, width: usize) -> (Shown, Vec<Span>) {
    let mut shown = Shown::all(parts);
    loop {
        let left = left_spans(parts, &shown);
        if spans_len(&left) + fixed_len(parts, &shown) <= width || !shown.drop_next() {
            return (shown, left);
        }
    }
}

fn fixed_len(parts: &Parts, shown: &Shown) -> usize {
    let remote_len = if shown.has(Part::Remote) {
        parts.remote.as_ref().map_or(0, |spans| spans_len(spans))
    } else {
        0
    };
    let pending_len = if shown.has(Part::Pending) {
        parts.pending.as_deref().map_or(0, char_len)
    } else {
        0
    };
    let hold_len = parts
        .hold
        .as_deref()
        .map(|reason| char_len(&hold_text(reason, shown.has(Part::Hint), parts.prefix)))
        .unwrap_or(0);
    remote_len + pending_len + hold_len
}

fn split_width(left_len: usize, hold_len: usize, width: usize) -> (usize, usize) {
    if left_len + hold_len > width {
        if hold_len >= width {
            (0, width)
        } else {
            (width - hold_len, hold_len)
        }
    } else {
        (left_len, hold_len)
    }
}

fn is_low_quota(quota: &QuotaInfo) -> bool {
    quota.limit_reached
        || quota.five_hour_left.is_some_and(|percent| percent <= 15)
        || quota.weekly_left.is_some_and(|percent| percent <= 15)
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

fn left_spans(parts: &Parts, shown: &Shown) -> Vec<Span> {
    let mut spans = vec![
        Span::plain(" "),
        Span::styled(NAME_OPEN, &parts.name, NAME_CLOSE),
        Span::plain(" "),
    ];
    spans.extend(
        shown
            .pick(Part::Confined, parts.confine)
            .map(confined_cell)
            .unwrap_or_default(),
    );
    spans.extend(
        shown
            .pick(Part::Activity, parts.activity)
            .map(activity_cell)
            .unwrap_or_default(),
    );
    spans.extend(
        shown
            .pick(Part::Quota, parts.quota.as_ref())
            .map(|(text, limit_reached)| quota_cell(text, *limit_reached))
            .unwrap_or_default(),
    );
    spans.extend(
        shown
            .pick(Part::Role, parts.role.as_deref())
            .map(|text| detail_cell(vec![Span::plain(text)]))
            .unwrap_or_default(),
    );
    spans.extend(
        shown
            .pick(Part::Team, parts.team.as_deref())
            .map(|text| detail_cell(vec![Span::plain(text)]))
            .unwrap_or_default(),
    );
    spans.extend(
        shown
            .pick(Part::Harness, parts.harness)
            .map(|harness| detail_cell(vec![Span::plain(harness.as_str())]))
            .unwrap_or_default(),
    );
    spans
}

fn confined_cell(auto: bool) -> Vec<Span> {
    let tag = if auto {
        "[confined auto]"
    } else {
        "[confined]"
    };
    vec![Span::styled("\x1b[2m", tag, "\x1b[22m"), Span::plain(" ")]
}

fn activity_cell(state: DotState) -> Vec<Span> {
    detail_cell(vec![
        Span::styled(activity_color(state), "●", BAR_FOREGROUND),
        Span::plain(" "),
        Span::plain(activity_word(state)),
    ])
}

fn quota_cell(text: &str, limit_reached: bool) -> Vec<Span> {
    detail_cell(vec![Span::styled(
        if limit_reached {
            "\x1b[38;5;203m"
        } else {
            "\x1b[38;5;221m"
        },
        text,
        BAR_FOREGROUND,
    )])
}

fn detail_cell(content: Vec<Span>) -> Vec<Span> {
    let mut spans = detail_prefix().to_vec();
    spans.extend(content);
    spans.push(Span::plain(" "));
    spans
}

fn remote_cell(remote: &RemoteStatus) -> (&'static str, &'static str) {
    match (remote.lease, remote.gateway) {
        (RemoteLease::None, _) => ("remote: no lease", "\x1b[38;5;244m"),
        (RemoteLease::Active, true) => ("remote on", "\x1b[38;5;78m"),
        (RemoteLease::Active, false) => ("remote: gateway off", "\x1b[38;5;209m"),
        (RemoteLease::Ended, _) => ("remote: lease ended", "\x1b[38;5;203m"),
    }
}

fn remote_spans(remote: RemoteStatus) -> Vec<Span> {
    let (text, color) = remote_cell(&remote);
    vec![
        Span::styled(color, "●", BAR_FOREGROUND),
        Span::plain(" "),
        Span::plain(format!("{text} ")),
    ]
}
