/// An action recognized while the session picker owns terminal input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerAction {
    Cancel,
    Move(isize),
    Page(isize),
    Home,
    End,
    Select,
    Toggle,
    Fold,
    Unfold,
    StartFilter,
    Char(u8),
    Backspace,
}

/// Incrementally parses the picker key sequences across terminal reads.
#[derive(Default)]
pub struct PickerParser {
    pending: Vec<u8>,
    filter_mode: bool,
}

impl PickerParser {
    /// Enables or disables the filter-mode byte mappings.
    pub fn set_filter_mode(&mut self, on: bool) {
        self.filter_mode = on;
    }

    /// Feeds bytes from one terminal read into the parser.
    pub fn feed(&mut self, bytes: &[u8], escape_alone: bool) -> Vec<PickerAction> {
        // shortcut: ESC-alone detection is intentionally one-read based.
        if escape_alone {
            self.pending.clear();
            return vec![PickerAction::Cancel];
        }
        let mut actions = Vec::new();
        for byte in bytes {
            if self.pending.len() == 1 {
                if *byte == b'[' || *byte == b'O' {
                    self.pending.push(*byte);
                    continue;
                }
                self.pending.clear();
            }
            if self.pending.is_empty() {
                if self.filter_mode {
                    if (0x20..=0x7e).contains(byte) {
                        actions.push(PickerAction::Char(*byte));
                        continue;
                    }
                    if *byte == 0x7f || *byte == 0x08 {
                        actions.push(PickerAction::Backspace);
                        continue;
                    }
                } else {
                    if *byte == b' ' {
                        actions.push(PickerAction::Toggle);
                        continue;
                    }
                    if *byte == b'h' {
                        actions.push(PickerAction::Fold);
                        continue;
                    }
                    if *byte == b'l' {
                        actions.push(PickerAction::Unfold);
                        continue;
                    }
                    if *byte == b'/' {
                        actions.push(PickerAction::StartFilter);
                        continue;
                    }
                }
                if *byte == b'q' {
                    actions.push(PickerAction::Cancel);
                    continue;
                }
                if *byte == b'j' {
                    actions.push(PickerAction::Move(1));
                    continue;
                }
                if *byte == b'k' {
                    actions.push(PickerAction::Move(-1));
                    continue;
                }
                if *byte == b'\r' {
                    actions.push(PickerAction::Select);
                    continue;
                }
                if *byte == 0x1b {
                    self.pending.push(*byte);
                    continue;
                }
            } else {
                self.pending.push(*byte);
            }
            if self.pending == b"\x1b[A" || self.pending == b"\x1bOA" {
                self.pending.clear();
                actions.push(PickerAction::Move(-1));
            } else if self.pending == b"\x1b[B" || self.pending == b"\x1bOB" {
                self.pending.clear();
                actions.push(PickerAction::Move(1));
            } else if self.pending == b"\x1b[5~" {
                self.pending.clear();
                actions.push(PickerAction::Page(-1));
            } else if self.pending == b"\x1b[6~" {
                self.pending.clear();
                actions.push(PickerAction::Page(1));
            } else if self.pending == b"\x1b[H"
                || self.pending == b"\x1bOH"
                || self.pending == b"\x1b[1~"
            {
                self.pending.clear();
                actions.push(PickerAction::Home);
            } else if self.pending == b"\x1b[F"
                || self.pending == b"\x1bOF"
                || self.pending == b"\x1b[4~"
            {
                self.pending.clear();
                actions.push(PickerAction::End);
            } else if self.pending == b"\x1b[C" || self.pending == b"\x1bOC" {
                self.pending.clear();
                actions.push(PickerAction::Unfold);
            } else if self.pending == b"\x1b[D" || self.pending == b"\x1bOD" {
                self.pending.clear();
                actions.push(PickerAction::Fold);
            } else if self.pending.len() > 3 || self.pending.first() != Some(&0x1b) {
                self.pending.clear();
            }
        }
        actions
    }
}

/// Keeps the selected row visible without jumping the window on every move.
///
/// The caller stores the returned offset and recomputes it after selection, refresh,
/// or resize changes so a visible window remains stable while the selection stays in view.
pub fn scroll_into_view(offset: usize, selection: usize, height: usize, total: usize) -> usize {
    if height == 0 || total <= height {
        return 0;
    }
    let offset = if selection < offset {
        selection
    } else if selection >= offset.saturating_add(height) {
        selection + 1 - height
    } else {
        offset
    };
    offset.min(total - height)
}

/// Computes a clamped picker selection for one navigation action.
pub fn target(selection: usize, action: PickerAction, height: usize, total: usize) -> usize {
    if total == 0 {
        return 0;
    }
    let max = total - 1;
    let bounded = selection.min(max);
    match action {
        PickerAction::Move(delta) => {
            apply_delta(bounded, delta.unsigned_abs(), delta.is_negative(), max)
        }
        PickerAction::Page(delta) => {
            let page = height.saturating_sub(1).max(1);
            let amount = delta.unsigned_abs().saturating_mul(page);
            apply_delta(bounded, amount, delta.is_negative(), max)
        }
        PickerAction::Home => 0,
        PickerAction::End => max,
        PickerAction::Cancel
        | PickerAction::Select
        | PickerAction::Toggle
        | PickerAction::Fold
        | PickerAction::Unfold
        | PickerAction::StartFilter
        | PickerAction::Char(_)
        | PickerAction::Backspace => selection,
    }
}

fn apply_delta(selection: usize, amount: usize, negative: bool, max: usize) -> usize {
    if negative {
        selection.saturating_sub(amount)
    } else {
        selection.saturating_add(amount).min(max)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryState {
    Working,
    Busy,
    Idle,
    Unknown,
    Exited,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerEntry {
    pub id: String,
    pub name: String,
    pub team: Option<String>,
    pub private: bool,
    pub state: EntryState,
    pub held: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    Header { key: Option<String>, text: String },
    Session { id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Attach(String),
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Selection {
    Header(Option<String>),
    Session(String),
}

#[derive(Clone)]
struct Section {
    key: Option<String>,
    private: bool,
    entries: Vec<PickerEntry>,
}

pub struct PickerView {
    entries: Vec<PickerEntry>,
    current: String,
    folds: std::collections::HashMap<Option<String>, bool>,
    selection: Option<Selection>,
    filter_mode: bool,
    query: String,
}

impl PickerView {
    pub fn new(entries: Vec<PickerEntry>, current: &str) -> Self {
        let mut view = Self {
            entries,
            current: current.to_owned(),
            folds: std::collections::HashMap::new(),
            selection: None,
            filter_mode: false,
            query: String::new(),
        };
        view.folds = view.default_folds();
        view.selection = view
            .entries
            .iter()
            .find(|entry| entry.id == current)
            .map(|entry| Selection::Session(entry.id.clone()))
            .or_else(|| view.lines().first().map(Self::selection_for_line));
        view
    }

    pub fn replace(&mut self, entries: Vec<PickerEntry>) {
        let old_selection = self.selection.clone();
        let old_index = self.selected_line();
        let old_team = old_selection
            .as_ref()
            .and_then(|selection| self.selection_team(selection));
        let old_folds = self.folds.clone();

        self.entries = entries;
        self.folds = self.default_folds();
        for (key, expanded) in old_folds {
            if self.folds.contains_key(&key) {
                self.folds.insert(key, expanded);
            }
        }

        if self.query.is_empty() {
            self.reconcile_empty(old_selection, old_index, old_team);
        } else {
            self.reconcile_filter();
        }
    }

    pub fn lines(&self) -> Vec<Line> {
        if !self.sectioned() {
            return self
                .entries
                .iter()
                .filter(|entry| self.matches(entry))
                .map(|entry| Line::Session {
                    id: entry.id.clone(),
                })
                .collect();
        }

        let mut lines = Vec::new();
        for section in self.sections() {
            let shown: Vec<_> = section
                .entries
                .iter()
                .filter(|entry| self.matches(entry))
                .collect();
            if shown.is_empty() && !self.query.is_empty() {
                continue;
            }
            let expanded =
                !self.query.is_empty() || self.folds.get(&section.key).copied().unwrap_or(true);
            let summary = if expanded {
                String::new()
            } else {
                Self::summary(&shown)
            };
            lines.push(Line::Header {
                key: section.key.clone(),
                text: Self::header_text(
                    &section.key,
                    section.private,
                    shown.len(),
                    expanded,
                    &summary,
                ),
            });
            if expanded {
                lines.extend(shown.into_iter().map(|entry| Line::Session {
                    id: entry.id.clone(),
                }));
            }
        }
        lines
    }

    pub fn selected_line(&self) -> Option<usize> {
        let selection = self.selection.as_ref()?;
        self.lines()
            .iter()
            .position(|line| &Self::selection_for_line(line) == selection)
    }

    pub fn selected_session(&self) -> Option<&str> {
        let Selection::Session(id) = self.selection.as_ref()? else {
            return None;
        };
        self.selected_line()
            .and_then(|line| match self.lines().get(line) {
                Some(Line::Session { id: selected }) if selected == id => Some(id.as_str()),
                _ => None,
            })
    }

    pub fn filter_mode(&self) -> bool {
        self.filter_mode
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn apply(&mut self, action: PickerAction, height: usize) -> Option<Outcome> {
        match action {
            PickerAction::Move(_)
            | PickerAction::Page(_)
            | PickerAction::Home
            | PickerAction::End => {
                let lines = self.lines();
                let selected = self.selected_line()?;
                let target = target(selected, action, height, lines.len());
                self.selection = lines.get(target).map(Self::selection_for_line);
            }
            PickerAction::Toggle | PickerAction::Fold | PickerAction::Unfold => {
                self.apply_fold(action);
            }
            PickerAction::StartFilter => {
                self.filter_mode = true;
                self.query.clear();
            }
            PickerAction::Char(byte) => {
                if self.filter_mode {
                    self.query.push(char::from(byte));
                    self.reconcile_filter();
                }
            }
            PickerAction::Backspace => {
                if self.filter_mode && !self.query.is_empty() {
                    let old_selection = self.selection.clone();
                    let old_index = self.selected_line();
                    let old_team = old_selection
                        .as_ref()
                        .and_then(|selection| self.selection_team(selection));
                    self.query.pop();
                    if self.query.is_empty() {
                        self.reconcile_empty(old_selection, old_index, old_team);
                    } else {
                        self.reconcile_filter();
                    }
                }
            }
            PickerAction::Select => {
                let line = self.selected_line()?;
                match self.lines().get(line) {
                    Some(Line::Session { id }) => return Some(Outcome::Attach(id.clone())),
                    Some(Line::Header { .. }) if self.query.is_empty() => {
                        self.apply_fold(PickerAction::Toggle);
                    }
                    _ => {}
                }
            }
            PickerAction::Cancel => {
                if self.filter_mode {
                    let old_selection = self.selection.clone();
                    let old_index = self.selected_line();
                    let old_team = old_selection
                        .as_ref()
                        .and_then(|selection| self.selection_team(selection));
                    self.query.clear();
                    self.filter_mode = false;
                    self.reconcile_empty(old_selection, old_index, old_team);
                } else {
                    return Some(Outcome::Cancel);
                }
            }
        }
        None
    }

    fn sectioned(&self) -> bool {
        self.entries.iter().any(|entry| entry.team.is_some())
    }

    fn sections(&self) -> Vec<Section> {
        if !self.sectioned() {
            return Vec::new();
        }
        let mut sections = Vec::new();
        for entry in &self.entries {
            let key = entry.team.clone();
            if let Some(section) = sections
                .iter_mut()
                .find(|section: &&mut Section| section.key == key)
            {
                section.entries.push(entry.clone());
                section.private |= entry.private;
            } else {
                sections.push(Section {
                    key,
                    private: entry.private,
                    entries: vec![entry.clone()],
                });
            }
        }
        sections.sort_by(|left, right| match (&left.key, &right.key) {
            (Some(left), Some(right)) => left.as_bytes().cmp(right.as_bytes()),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        });
        sections
    }

    fn default_folds(&self) -> std::collections::HashMap<Option<String>, bool> {
        let sections = self.sections();
        let named = sections
            .iter()
            .filter(|section| section.key.is_some())
            .count();
        sections
            .into_iter()
            .map(|section| {
                let expanded =
                    named <= 2 || section.entries.iter().any(|entry| entry.id == self.current);
                (section.key, expanded)
            })
            .collect()
    }

    fn matches(&self, entry: &PickerEntry) -> bool {
        self.query.is_empty()
            || ascii_contains(&entry.id, &self.query)
            || ascii_contains(&entry.name, &self.query)
            || entry
                .team
                .as_deref()
                .is_some_and(|team| ascii_contains(team, &self.query))
    }

    fn summary(entries: &[&PickerEntry]) -> String {
        let mut parts = Vec::new();
        let working = entries
            .iter()
            .filter(|entry| entry.state == EntryState::Working)
            .count();
        if working > 0 {
            parts.push(format!("{working} working"));
        }
        let busy = entries
            .iter()
            .filter(|entry| entry.state == EntryState::Busy)
            .count();
        if busy > 0 {
            parts.push(format!("{busy} busy"));
        }
        let idle = entries
            .iter()
            .filter(|entry| entry.state == EntryState::Idle)
            .count();
        if idle > 0 {
            parts.push(format!("{idle} idle"));
        }
        let held = entries
            .iter()
            .filter(|entry| entry.state != EntryState::Exited && entry.held)
            .count();
        if held > 0 {
            parts.push(format!("{held} held"));
        }
        let exited = entries
            .iter()
            .filter(|entry| entry.state == EntryState::Exited)
            .count();
        if exited > 0 {
            parts.push(format!("{exited} exited"));
        }
        parts.join(", ")
    }

    fn header_text(
        key: &Option<String>,
        private: bool,
        count: usize,
        expanded: bool,
        summary: &str,
    ) -> String {
        let mut name = key.as_deref().unwrap_or("(no team)").to_owned();
        if key.is_some() && private {
            name.push_str(" (private)");
        }
        let marker = if expanded { '-' } else { '+' };
        let mut text = format!("[{marker}] {name} ({count})");
        if !expanded && !summary.is_empty() {
            text.push_str(": ");
            text.push_str(summary);
        }
        text
    }

    fn selection_for_line(line: &Line) -> Selection {
        match line {
            Line::Header { key, .. } => Selection::Header(key.clone()),
            Line::Session { id } => Selection::Session(id.clone()),
        }
    }

    fn selection_team(&self, selection: &Selection) -> Option<Option<String>> {
        match selection {
            Selection::Header(key) => Some(key.clone()),
            Selection::Session(id) => self
                .entries
                .iter()
                .find(|entry| entry.id == *id)
                .map(|entry| entry.team.clone()),
        }
    }

    fn contains_selection(&self, lines: &[Line], selection: &Selection) -> bool {
        lines
            .iter()
            .any(|line| &Self::selection_for_line(line) == selection)
    }

    fn selection_at(lines: &[Line], index: Option<usize>) -> Option<Selection> {
        index
            .and_then(|index| lines.get(index.min(lines.len().saturating_sub(1))))
            .map(Self::selection_for_line)
    }

    fn first_session_or_line(lines: &[Line]) -> Option<Selection> {
        lines
            .iter()
            .find(|line| matches!(line, Line::Session { .. }))
            .or_else(|| lines.first())
            .map(Self::selection_for_line)
    }

    fn section_exists(&self, key: &Option<String>) -> bool {
        self.sections().iter().any(|section| &section.key == key)
    }

    fn reconcile_filter(&mut self) {
        let lines = self.lines();
        if self
            .selection
            .as_ref()
            .is_some_and(|selection| self.contains_selection(&lines, selection))
        {
            return;
        }
        self.selection = Self::first_session_or_line(&lines);
    }

    fn reconcile_empty(
        &mut self,
        old_selection: Option<Selection>,
        old_index: Option<usize>,
        old_team: Option<Option<String>>,
    ) {
        let lines = self.lines();
        if let Some(selection) = old_selection.as_ref() {
            if self.contains_selection(&lines, selection) {
                self.selection = Some(selection.clone());
                return;
            }
            if let Selection::Session(_) = selection {
                let team = old_team
                    .or_else(|| self.selection_team(selection))
                    .filter(|key| self.section_exists(key));
                if let Some(team) = team {
                    let folded = !self.folds.get(&team).copied().unwrap_or(true);
                    if folded {
                        self.selection = Some(Selection::Header(team));
                        return;
                    }
                }
            }
        }
        self.selection = Self::selection_at(&lines, old_index);
        if self.selection.is_none() && old_selection.is_none() {
            self.selection = lines.first().map(Self::selection_for_line);
        }
    }

    fn apply_fold(&mut self, action: PickerAction) {
        if !self.query.is_empty() {
            return;
        }
        match self.selection.clone() {
            Some(Selection::Header(key)) => {
                let expanded = self.folds.get(&key).copied().unwrap_or(true);
                let next = match action {
                    PickerAction::Toggle => !expanded,
                    PickerAction::Fold => false,
                    PickerAction::Unfold => true,
                    _ => expanded,
                };
                if self.section_exists(&key) {
                    self.folds.insert(key, next);
                }
            }
            Some(Selection::Session(id)) if action == PickerAction::Fold => {
                if let Some(key) = self
                    .entries
                    .iter()
                    .find(|entry| entry.id == id)
                    .and_then(|entry| entry.team.clone())
                {
                    if self.sectioned() && self.section_exists(&Some(key.clone())) {
                        self.folds.insert(Some(key.clone()), false);
                        self.selection = Some(Selection::Header(Some(key)));
                    }
                } else if self.sectioned() {
                    self.folds.insert(None, false);
                    self.selection = Some(Selection::Header(None));
                }
            }
            _ => {}
        }
    }
}

fn ascii_contains(value: &str, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    value
        .as_bytes()
        .windows(query.len())
        .any(|window| window.eq_ignore_ascii_case(query.as_bytes()))
}
