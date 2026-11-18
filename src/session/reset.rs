use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use crate::delivery;
use crate::harness::{self, Harness};
use crate::messaging;

use super::{DeliveryOutcome, Hold, Session, State};

const RESET_SETTLE: Duration = Duration::from_millis(1500); // shortcut: fixed settle; replace with a per-harness completion marker if a harness reports readiness early or needs longer.
const STALE_DRAFT_MARGIN: Duration = Duration::from_secs(2); // shortcut: fixed margin; replace with a per-keystroke acknowledgement if OMP polling becomes slower.
const RESET_POLL: Duration = Duration::from_millis(100);
const RESET_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug)]
pub(crate) struct ResetError {
    pub(crate) code: &'static str,
    pub(crate) step: Option<(usize, usize)>,
    pub(crate) message: String,
}

pub(crate) struct ResetGuard<'a>(&'a Session);

impl Drop for ResetGuard<'_> {
    fn drop(&mut self) {
        self.0.resetting.store(false, Ordering::Release);
        self.0.message_notify.notify_one();
    }
}

impl Session {
    fn reset_native_reason(&self) -> Option<(&'static str, String)> {
        if self.harness != Harness::Omp {
            return None;
        }
        let state = self.lock();
        let timing = Self::native_timing(&state.native);
        if state.native.refused {
            Some((
                messaging::code::NOT_READY,
                format!("native bridge refused{timing}"),
            ))
        } else if !state.native.connected {
            Some((
                messaging::code::NOT_READY,
                format!("native bridge not connected{timing}"),
            ))
        } else if state.native.draft {
            Some((
                messaging::code::DRAFT_PRESENT,
                format!("native draft=true{timing}"),
            ))
        } else if !state.native.idle || state.native.in_flight || state.native.waiting.is_some() {
            let diagnostics = state
                .native
                .diagnostics
                .map(|(queue, host_pending)| {
                    format!(" ext_queue={queue} ext_pending={host_pending}")
                })
                .unwrap_or_default();
            Some((
                messaging::code::BUSY,
                format!(
                    "native busy: idle={} in_flight={} waiting={}{}{}",
                    state.native.idle,
                    state.native.in_flight,
                    state.native.waiting.is_some(),
                    timing,
                    diagnostics
                ),
            ))
        } else {
            None
        }
    }

    fn stale_omp_draft(state: &State) -> bool {
        state.hold == Some(Hold::HumanDraft)
            && state.restore.is_none()
            && state.native.connected
            && !state.native.refused
            && !state.native.draft
            && state.native.idle
            && state
                .last_human_input
                .is_none_or(|input| input.elapsed() >= STALE_DRAFT_MARGIN)
    }

    fn stale_claude_draft(state: &State) -> bool {
        state.hold == Some(Hold::HumanDraft)
            && state.restore.is_none()
            && state
                .last_human_input
                .is_none_or(|input| input.elapsed() >= STALE_DRAFT_MARGIN)
            && harness::claude_composer_empty(
                &state.emulator.screen(),
                state.emulator.is_scrolled(),
            )
    }
    fn stale_agy_draft(state: &State) -> bool {
        state.hold == Some(Hold::HumanDraft)
            && state.restore.is_none()
            && state
                .last_human_input
                .is_none_or(|input| input.elapsed() >= STALE_DRAFT_MARGIN)
            && harness::agy_composer_idle(&state.emulator.screen(), state.emulator.is_scrolled())
    }

    pub(super) fn draft_is_stale(&self, state: &State) -> bool {
        match self.harness {
            Harness::Omp => Self::stale_omp_draft(state),
            Harness::Claude => Self::stale_claude_draft(state),
            Harness::Agy => Self::stale_agy_draft(state),
            Harness::Generic | Harness::Codex => false,
        }
    }

    fn clear_stale_draft(&self) {
        let mut state = self.lock();
        if self.draft_is_stale(&state) {
            state.hold = None;
        }
    }

    pub(crate) fn reset_gate_preview(&self) -> Option<(&'static str, String)> {
        if self.resetting() {
            return Some((messaging::code::BUSY, "reset is already running".to_owned()));
        }
        self.reset_gate_decision()
    }

    pub(super) fn reset_gate_reason(&self) -> Option<(&'static str, String)> {
        self.clear_stale_draft();
        self.reset_gate_decision()
    }

    fn reset_gate_decision(&self) -> Option<(&'static str, String)> {
        if self.exit_code().is_some() {
            return Some((messaging::code::EXITED, "exited".to_owned()));
        }
        let gate = {
            let state = self.lock();
            let hold = if self.draft_is_stale(&state) {
                None
            } else {
                state.hold
            };
            if let Some(hold) = hold {
                let code = match hold {
                    Hold::HumanDraft => messaging::code::DRAFT_PRESENT,
                    Hold::UnsubmittedEnvelope | Hold::CorruptedSubmissions => messaging::code::HELD,
                };
                Some((code, format!("hold {}", delivery::hold_name(hold))))
            } else if !harness::ready(
                self.harness,
                &state.emulator.screen(),
                state.emulator.is_scrolled(),
            ) {
                Some((messaging::code::NOT_READY, "screen not ready".to_owned()))
            } else if self.harness == Harness::Agy
                && !harness::agy_composer_idle(
                    &state.emulator.screen(),
                    state.emulator.is_scrolled(),
                )
            {
                Some((
                    messaging::code::DRAFT_PRESENT,
                    "composer not empty".to_owned(),
                ))
            } else if self.harness == Harness::Agy
                && harness::agy_turn_in_flight(&state.emulator.screen())
            {
                Some((messaging::code::BUSY, "turn in flight".to_owned()))
            } else {
                None
            }
        };
        gate.or_else(|| self.reset_native_reason())
    }

    fn reset_error(
        code: &'static str,
        step: Option<(usize, usize)>,
        message: impl Into<String>,
    ) -> ResetError {
        ResetError {
            code,
            step,
            message: message.into(),
        }
    }

    pub(crate) fn begin_reset(&self) -> Result<ResetGuard<'_>, ResetError> {
        self.resetting
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| {
                Self::reset_error(messaging::code::BUSY, None, "reset is already running")
            })?;
        self.message_notify.notify_one();
        Ok(ResetGuard(self))
    }

    pub(crate) async fn reset_sequence(&self) -> Result<u32, ResetError> {
        let _guard = self.begin_reset()?;
        let steps = if self.reset.is_empty() {
            vec!["/clear".to_owned()]
        } else {
            self.reset.clone()
        };
        let total = steps.len();
        for (index, step) in steps.iter().enumerate() {
            let number = index + 1;
            if let Some((code, source)) = self.reset_gate_reason() {
                return Err(Self::reset_error(
                    code,
                    Some((number, total)),
                    format!("reset gate rejected step {number} of {total}: {source}"),
                ));
            }
            let Some(delivery) = self.begin_delivery().await else {
                let (code, source) = self
                    .reset_gate_reason()
                    .unwrap_or((messaging::code::HELD, "begin_delivery refused".to_owned()));
                return Err(Self::reset_error(
                    code,
                    Some((number, total)),
                    format!("reset gate rejected step {number} of {total}: {source}"),
                ));
            };
            match delivery
                .submit(messaging::paste_bytes(step), messaging::PASTE_GAP)
                .await
            {
                DeliveryOutcome::Submitted => {}
                DeliveryOutcome::Unsubmitted(reason) => {
                    return Err(Self::reset_error(
                        messaging::code::HELD,
                        Some((number, total)),
                        format!("step {number} of {total} was not submitted: {reason}"),
                    ));
                }
                DeliveryOutcome::Failed(reason) => {
                    return Err(Self::reset_error(
                        messaging::code::WRITE_FAILED,
                        Some((number, total)),
                        format!("step {number} of {total} failed: {reason}"),
                    ));
                }
            }
            let deadline = Instant::now() + RESET_TIMEOUT;
            tokio::time::sleep(RESET_SETTLE).await;
            loop {
                if self.exit_code().is_some() {
                    return Err(Self::reset_error(
                        messaging::code::EXITED,
                        Some((number, total)),
                        format!("session exited during step {number} of {total}"),
                    ));
                }
                if self.reset_gate_reason().is_none() {
                    break;
                }
                if Instant::now() >= deadline {
                    return Err(Self::reset_error(
                        messaging::code::STEP_TIMEOUT,
                        Some((number, total)),
                        format!("step {number} of {total} did not become ready"),
                    ));
                }
                tokio::time::sleep(RESET_POLL).await;
            }
        }
        Ok(total as u32)
    }
}
