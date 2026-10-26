use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Thresholds {
    pub working_after: Option<Duration>,
    pub idle_after: Option<Duration>,
}

#[derive(Debug, Clone)]
pub struct Tracker {
    thresholds: Thresholds,
    current: Option<bool>,
    current_since: Option<Instant>,
    settled: Option<bool>,
    reference: Option<bool>,
}

impl Tracker {
    pub fn new(thresholds: Thresholds) -> Self {
        Self {
            thresholds,
            current: None,
            current_since: None,
            settled: None,
            reference: None,
        }
    }

    pub fn observe(&mut self, now: Instant, working: bool) -> Option<bool> {
        match self.current {
            Some(current) if current == working => {}
            _ => {
                self.current = Some(working);
                self.current_since = Some(now);
            }
        }

        let since = self.current_since?;
        let threshold = if working {
            self.thresholds.working_after
        } else {
            self.thresholds.idle_after
        };
        if now.saturating_duration_since(since) < threshold.unwrap_or_default() {
            return self.pending_announcement();
        }
        if self.settled != Some(working) {
            self.settled = Some(working);
            if self.reference.is_none() || threshold.is_none() {
                self.reference = Some(working);
            }
        }
        self.pending_announcement()
    }

    pub fn announced(&mut self, state: bool) {
        self.reference = Some(state);
    }

    fn pending_announcement(&self) -> Option<bool> {
        let settled = self.settled?;
        if self.reference == Some(settled) {
            return None;
        }
        let threshold = if settled {
            self.thresholds.working_after
        } else {
            self.thresholds.idle_after
        };
        threshold.map(|_| settled)
    }
}
