/// An action recognized while the session picker owns terminal input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerAction {
    Cancel,
    Move(isize),
    Page(isize),
    Home,
    End,
    Select,
}

/// Incrementally parses the picker key sequences across terminal reads.
#[derive(Default)]
pub struct PickerParser {
    pending: Vec<u8>,
}

impl PickerParser {
    /// Feeds bytes from one terminal read into the parser.
    pub fn feed(&mut self, bytes: &[u8], escape_alone: bool) -> Vec<PickerAction> {
        // shortcut: ESC-alone detection is intentionally one-read based.
        if escape_alone {
            self.pending.clear();
            return vec![PickerAction::Cancel];
        }
        let mut actions = Vec::new();
        for byte in bytes {
            if self.pending.is_empty() && *byte == b'q' {
                actions.push(PickerAction::Cancel);
                continue;
            }
            if self.pending.is_empty() && *byte == b'j' {
                actions.push(PickerAction::Move(1));
                continue;
            }
            if self.pending.is_empty() && *byte == b'k' {
                actions.push(PickerAction::Move(-1));
                continue;
            }
            if self.pending.is_empty() && *byte == b'\r' {
                actions.push(PickerAction::Select);
                continue;
            }
            self.pending.push(*byte);
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
        PickerAction::Cancel | PickerAction::Select => selection,
    }
}

fn apply_delta(selection: usize, amount: usize, negative: bool, max: usize) -> usize {
    if negative {
        selection.saturating_sub(amount)
    } else {
        selection.saturating_add(amount).min(max)
    }
}
