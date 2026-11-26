use crate::wire::TeamMember;

use super::dot::{activity_color, dot_state};
use super::spans::{
    BAR_BACKGROUND, BAR_FOREGROUND, NAME_CLOSE, NAME_OPEN, Span, char_len, detail_prefix,
    render_separator, render_spans, sanitize, spans_len,
};

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
