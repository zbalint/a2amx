use crate::harness::Harness;
use crate::quota;
use crate::wire::{Activity, QuotaInfo, StatusInfo, TeamMember};

const BAR_FOREGROUND: &str = "\x1b[38;5;252m";
const BAR_BACKGROUND: &str = "\x1b[38;5;252;48;5;236m";
const NAME_OPEN: &str = "\x1b[1m";
const NAME_CLOSE: &str = "\x1b[22m";
const SEPARATOR_OPEN: &str = "\x1b[38;5;240m";
const ALERT_OPEN: &str = "\x1b[0m\x1b[1;37;41m";

#[derive(Clone)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamRegion {
    pub first_col: u16,
    pub last_col: u16,
    pub target: TeamTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TeamTarget {
    Session(String),
    Picker,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamLine {
    pub bytes: Vec<u8>,
    pub regions: Vec<TeamRegion>,
}

/// Bytes that draw the team member line on terminal row 1, leaving the cursor where it was.
pub fn render_team_line(
    current_id: &str,
    team: &str,
    members: &[TeamMember],
    cols: u16,
) -> TeamLine {
    if cols < 2 {
        return TeamLine {
            bytes: Vec::new(),
            regions: Vec::new(),
        };
    }
    let width = usize::from(cols - 1);
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"\x1b7\x1b[1;1H\x1b[0m\x1b[2K");
    bytes.extend_from_slice(BAR_BACKGROUND.as_bytes());
    if members.is_empty() {
        bytes.extend_from_slice(" ".repeat(width).as_bytes());
        bytes.extend_from_slice(b"\x1b[0m\x1b8");
        return TeamLine {
            bytes,
            regions: Vec::new(),
        };
    }

    let label = format!(" team {} ", clipped_name(team));
    let mut prefix = vec![Span::plain(label.clone())];
    prefix.extend(detail_prefix());
    let prefix_len = spans_len(&prefix);
    let member_spans = members
        .iter()
        .map(|member| {
            let state = dot_state(member.activity, member.held, member.starting, member.exited);
            let color = activity_color(state);
            let name = clipped_name(&sanitize(&member.name));
            let mut spans = vec![Span::styled(color, "●", BAR_FOREGROUND), Span::plain(" ")];
            if member.id == current_id {
                spans.push(Span::styled(NAME_OPEN, name, NAME_CLOSE));
            } else {
                spans.push(Span::plain(name));
            }
            spans.push(Span::plain("  "));
            spans
        })
        .collect::<Vec<_>>();
    let mut kept = vec![true; members.len()];
    let mut dropped = 0usize;
    loop {
        let members_len = kept
            .iter()
            .zip(&member_spans)
            .filter(|(is_kept, _)| **is_kept)
            .map(|(_, spans)| spans_len(spans))
            .sum::<usize>();
        let suffix_len = if dropped == 0 {
            0
        } else {
            char_len(&format!(" +{dropped} "))
        };
        if prefix_len + members_len + suffix_len <= width {
            break;
        }
        let Some(index) = kept
            .iter()
            .enumerate()
            .rev()
            .find(|(index, is_kept)| **is_kept && members[*index].id != current_id)
            .map(|(index, _)| index)
        else {
            break;
        };
        kept[index] = false;
        dropped += 1;
    }

    let mut content = prefix;
    let mut regions = Vec::new();
    add_team_region(&mut regions, 1, prefix_len, TeamTarget::Picker, width);
    let mut column = prefix_len + 1;
    for (member, (is_kept, spans)) in members.iter().zip(kept.iter().zip(&member_spans)) {
        if !*is_kept {
            continue;
        }
        let length = spans_len(spans);
        add_team_region(
            &mut regions,
            column,
            length,
            TeamTarget::Session(member.id.clone()),
            width,
        );
        content.extend(spans.iter().cloned());
        column += length;
    }
    if dropped > 0 {
        let suffix = format!(" +{dropped} ");
        let suffix_len = char_len(&suffix);
        add_team_region(&mut regions, column, suffix_len, TeamTarget::Picker, width);
        content.push(Span::plain(suffix));
        column += suffix_len;
    }
    let rendered = render_spans(&content, width);
    bytes.extend_from_slice(rendered.as_bytes());
    if column <= width + 1 {
        bytes.extend_from_slice(" ".repeat(width.saturating_sub(column - 1)).as_bytes());
    }
    bytes.extend_from_slice(b"\x1b[0m\x1b8");
    TeamLine { bytes, regions }
}

fn clipped_name(name: &str) -> String {
    let name = sanitize(name);
    if name.chars().count() <= 16 {
        return name;
    }
    let mut clipped = name.chars().take(15).collect::<String>();
    clipped.push('…');
    clipped
}

fn add_team_region(
    regions: &mut Vec<TeamRegion>,
    first: usize,
    length: usize,
    target: TeamTarget,
    width: usize,
) {
    if length == 0 || first > width {
        return;
    }
    let last = (first + length - 1).min(width);
    regions.push(TeamRegion {
        first_col: first as u16,
        last_col: last as u16,
        target,
    });
}

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
    let hold_reason = info.and_then(|info| info.hold.as_deref()).map(sanitize);
    let mut show_activity = activity.is_some();
    let mut show_quota = quota.is_some();
    let mut show_role = role.is_some();
    let mut show_team = team.is_some();
    let mut show_harness = harness.is_some();
    let mut show_sandbox = info.is_some_and(|info| info.confined);
    let mut show_pending = pending.is_some();
    let mut show_hint = hold_reason
        .as_deref()
        .is_some_and(|reason| hint_allowed(reason, prefix));

    loop {
        let left = left_spans(
            &name,
            show_sandbox,
            show_activity.then_some(activity).flatten(),
            show_quota
                .then(|| quota.as_ref().map(|(text, limit)| (text.as_str(), *limit)))
                .flatten(),
            show_role.then_some(role.as_deref()).flatten(),
            show_team.then_some(team.as_deref()).flatten(),
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
        if show_team {
            show_team = false;
        } else if show_harness {
            show_harness = false;
        } else if show_role {
            show_role = false;
        } else if show_quota {
            show_quota = false;
        } else if show_activity {
            show_activity = false;
        } else if show_pending {
            show_pending = false;
        } else if show_hint {
            show_hint = false;
        } else if show_sandbox {
            show_sandbox = false;
        } else {
            break;
        }
    }

    let left = left_spans(
        &name,
        show_sandbox,
        show_activity.then_some(activity).flatten(),
        show_quota
            .then(|| quota.as_ref().map(|(text, limit)| (text.as_str(), *limit)))
            .flatten(),
        show_role.then_some(role.as_deref()).flatten(),
        show_team.then_some(team.as_deref()).flatten(),
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
        bytes.extend_from_slice(render_separator(rows - 1, width).as_slice());
    }
    bytes.extend_from_slice(format!("\x1b[{rows};1H\x1b[0m{BAR_BACKGROUND}").as_bytes());
    bytes.extend_from_slice(left.as_bytes());
    bytes.extend_from_slice(padding.as_bytes());
    bytes.extend_from_slice(pending.as_bytes());
    bytes.extend_from_slice(hold.as_bytes());
    bytes.extend_from_slice(b"\x1b[0m\x1b8");
    bytes
}

fn render_separator(row: u16, width: usize) -> Vec<u8> {
    let mut bytes = format!("\x1b[{row};1H\x1b[0m\x1b[2K{SEPARATOR_OPEN}").into_bytes();
    for _ in 0..width {
        bytes.extend_from_slice("─".as_bytes());
    }
    bytes.extend_from_slice(b"\x1b[0m");
    bytes
}

/// Bytes that draw or clear the team separator on terminal row 2.
pub fn render_team_rule(cols: u16, drawn: bool) -> Vec<u8> {
    if cols < 2 {
        return Vec::new();
    }
    let width = usize::from(cols - 1);
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"\x1b7");
    if drawn {
        bytes.extend(render_separator(2, width));
    } else {
        bytes.extend_from_slice(b"\x1b[2;1H\x1b[0m\x1b[2K");
    }
    bytes.extend_from_slice(b"\x1b8");
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
    sandbox: bool,
    activity: Option<DotState>,
    quota: Option<(&str, bool)>,
    role: Option<&str>,
    team: Option<&str>,
    harness: Option<Harness>,
) -> Vec<Span> {
    let mut spans = vec![
        Span::plain(" "),
        Span::styled(NAME_OPEN, name, NAME_CLOSE),
        Span::plain(" "),
    ];
    if sandbox {
        spans.push(Span::styled("\x1b[2m", "[sandbox]", "\x1b[22m"));
        spans.push(Span::plain(" "));
    }
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
    if let Some(role) = role {
        spans.extend(detail_prefix());
        spans.push(Span::plain(role));
        spans.push(Span::plain(" "));
    }
    if let Some(team) = team {
        spans.extend(detail_prefix());
        spans.push(Span::plain(team));
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

#[derive(Clone, Copy)]
enum DotState {
    Idle,
    Working,
    Starting,
    Resetting,
    Held,
    Exited,
}

fn dot_state(activity: Option<Activity>, held: bool, starting: bool, exited: bool) -> DotState {
    if exited || activity.is_none() {
        DotState::Exited
    } else if held {
        DotState::Held
    } else if starting {
        DotState::Starting
    } else {
        match activity.unwrap_or(Activity::Idle) {
            Activity::Idle => DotState::Idle,
            Activity::Working => DotState::Working,
            Activity::Busy => DotState::Resetting,
        }
    }
}

fn activity_word(state: DotState) -> &'static str {
    match state {
        DotState::Idle => "idle",
        DotState::Working => "working",
        DotState::Starting => "starting",
        DotState::Resetting => "resetting",
        DotState::Held | DotState::Exited => "",
    }
}

fn activity_color(state: DotState) -> &'static str {
    match state {
        DotState::Idle => "\x1b[38;5;78m",
        DotState::Working => "\x1b[38;5;75m",
        DotState::Starting => "\x1b[38;5;221m",
        DotState::Resetting => "\x1b[38;5;209m",
        DotState::Held => "\x1b[38;5;203m",
        DotState::Exited => "\x1b[38;5;244m",
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
