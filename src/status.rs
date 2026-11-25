use crate::harness::Harness;
use crate::quota;
use crate::wire::{Activity, QuotaInfo, RemoteLease, RemoteStatus, StatusInfo, TeamMember};

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
    if members.is_empty() {
        return blank_team_line(width);
    }
    let mut bytes = team_line_open();

    let label = format!(" team {} ", clipped_name(team));
    let mut prefix = vec![Span::plain(label)];
    prefix.extend(detail_prefix());
    let prefix_len = spans_len(&prefix);
    let cells = members
        .iter()
        .map(|member| member_cell(member, member.id == current_id))
        .collect::<Vec<_>>();
    let room = width.saturating_sub(prefix_len);
    let kept = fit_members(&cells, members, current_id, room);
    let dropped = kept.iter().filter(|is_kept| !**is_kept).count();
    let selected = members
        .iter()
        .zip(kept.iter().zip(cells.iter()))
        .filter_map(|(member, (is_kept, spans))| (*is_kept).then_some((member, spans.as_slice())))
        .collect::<Vec<_>>();
    let TeamLayout {
        content,
        regions,
        column,
    } = lay_out_team(prefix, &selected, dropped, width);
    bytes.extend_from_slice(render_spans(&content, width).as_bytes());
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

fn team_line_open() -> Vec<u8> {
    let mut bytes = b"\x1b7\x1b[1;1H\x1b[0m\x1b[2K".to_vec();
    bytes.extend_from_slice(BAR_BACKGROUND.as_bytes());
    bytes
}

fn blank_team_line(width: usize) -> TeamLine {
    let mut bytes = team_line_open();
    bytes.extend_from_slice(" ".repeat(width).as_bytes());
    bytes.extend_from_slice(b"\x1b[0m\x1b8");
    TeamLine {
        bytes,
        regions: Vec::new(),
    }
}

fn member_cell(member: &TeamMember, is_current: bool) -> Vec<Span> {
    let state = dot_state(member.activity, member.held, member.starting, member.exited);
    let color = activity_color(state);
    let name = clipped_name(&sanitize(&member.name));
    let mut spans = vec![Span::styled(color, "●", BAR_FOREGROUND), Span::plain(" ")];
    if is_current {
        spans.push(Span::styled(NAME_OPEN, name, NAME_CLOSE));
    } else {
        spans.push(Span::plain(name));
    }
    spans.push(Span::plain("  "));
    spans
}

fn fit_members(
    cells: &[Vec<Span>],
    members: &[TeamMember],
    current_id: &str,
    room: usize,
) -> Vec<bool> {
    let mut kept = vec![true; members.len()];
    let mut dropped = 0usize;
    loop {
        let members_len = kept
            .iter()
            .zip(cells)
            .filter(|(is_kept, _)| **is_kept)
            .map(|(_, spans)| spans_len(spans))
            .sum::<usize>();
        let suffix_len = if dropped == 0 {
            0
        } else {
            char_len(&format!(" +{dropped} "))
        };
        if members_len + suffix_len <= room {
            return kept;
        }
        let Some(index) = kept
            .iter()
            .enumerate()
            .rev()
            .find(|(index, is_kept)| **is_kept && members[*index].id != current_id)
            .map(|(index, _)| index)
        else {
            return kept;
        };
        kept[index] = false;
        dropped += 1;
    }
}

struct TeamLayout {
    content: Vec<Span>,
    regions: Vec<TeamRegion>,
    column: usize,
}

fn lay_out_team(
    prefix: Vec<Span>,
    kept: &[(&TeamMember, &[Span])],
    dropped: usize,
    width: usize,
) -> TeamLayout {
    let prefix_len = spans_len(&prefix);
    let mut content = prefix;
    let mut book = RegionBook::new(width);
    book.add(1, prefix_len, TeamTarget::Picker);
    let mut column = prefix_len + 1;
    for (member, spans) in kept {
        let length = spans_len(spans);
        book.add(column, length, TeamTarget::Session(member.id.clone()));
        content.extend(spans.iter().cloned());
        column += length;
    }
    if dropped > 0 {
        let suffix = format!(" +{dropped} ");
        let suffix_len = char_len(&suffix);
        book.add(column, suffix_len, TeamTarget::Picker);
        content.push(Span::plain(suffix));
        column += suffix_len;
    }
    TeamLayout {
        content,
        regions: book.into_regions(),
        column,
    }
}

struct RegionBook {
    regions: Vec<TeamRegion>,
    width: usize,
}

impl RegionBook {
    fn new(width: usize) -> Self {
        Self {
            regions: Vec::new(),
            width,
        }
    }

    fn add(&mut self, first: usize, length: usize, target: TeamTarget) {
        if length == 0 || first > self.width {
            return;
        }
        let last = (first + length - 1).min(self.width);
        self.regions.push(TeamRegion {
            first_col: first as u16,
            last_col: last as u16,
            target,
        });
    }

    fn into_regions(self) -> Vec<TeamRegion> {
        self.regions
    }
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
    Waiting,
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
            Activity::Waiting => DotState::Waiting,
            Activity::Busy => DotState::Resetting,
        }
    }
}

fn activity_word(state: DotState) -> &'static str {
    match state {
        DotState::Idle => "idle",
        DotState::Working => "working",
        DotState::Waiting => "waiting",
        DotState::Starting => "starting",
        DotState::Resetting => "resetting",
        DotState::Held | DotState::Exited => "",
    }
}

fn activity_color(state: DotState) -> &'static str {
    match state {
        DotState::Idle => "\x1b[38;5;78m",
        DotState::Working => "\x1b[38;5;75m",
        DotState::Waiting => "\x1b[38;5;141m",
        DotState::Starting => "\x1b[38;5;221m",
        DotState::Resetting => "\x1b[38;5;209m",
        DotState::Held => "\x1b[38;5;203m",
        DotState::Exited => "\x1b[38;5;244m",
    }
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
