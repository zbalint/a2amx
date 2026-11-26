use crate::wire::Activity;

#[derive(Clone, Copy)]
pub(super) enum DotState {
    Idle,
    Working,
    Waiting,
    Starting,
    Resetting,
    Held,
    Exited,
}

pub(super) fn dot_state(
    activity: Option<Activity>,
    held: bool,
    starting: bool,
    exited: bool,
) -> DotState {
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

pub(super) fn activity_word(state: DotState) -> &'static str {
    match state {
        DotState::Idle => "idle",
        DotState::Working => "working",
        DotState::Waiting => "waiting",
        DotState::Starting => "starting",
        DotState::Resetting => "resetting",
        DotState::Held | DotState::Exited => "",
    }
}

pub(super) fn activity_color(state: DotState) -> &'static str {
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
