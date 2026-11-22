//! Turning wire views into CLI text: aligned session and message tables and the explain lines.

use std::time::Duration;

use crate::messaging::display_duration;
use crate::quota;
use crate::wire::{Activity, ExplainInfo, MessageInfo, SessionSummary};

pub fn session_confinement_mode(session: &SessionSummary) -> &'static str {
    session
        .confine
        .as_ref()
        .map_or("off", |info| if info.auto { "auto" } else { "manual" })
}
const SESSION_HEADERS: [&str; 13] = [
    "ID", "NAME", "HARNESS", "CONFINE", "STATE", "UPTIME", "ACTIVITY", "IN-STATE", "ATTACHED",
    "PENDING", "HELD", "QUOTA", "SIZE",
];
const DETAIL_HEADERS: [&str; 15] = [
    "ID", "NAME", "HARNESS", "CONFINE", "STATE", "UPTIME", "ACTIVITY", "IN-STATE", "ATTACHED",
    "PENDING", "HELD", "QUOTA", "SIZE", "CWD", "COMMAND",
];
const MESSAGE_HEADERS: [&str; 6] = ["ID", "FROM", "TO", "STATE", "DETAIL", "SUBJECT"];
pub(crate) fn session_value_rows(sessions: &[SessionSummary]) -> Vec<[String; 10]> {
    sessions
        .iter()
        .map(|session| {
            [
                session.id.clone(),
                session.name.clone().unwrap_or_else(|| "-".to_owned()),
                session.harness.as_str().to_owned(),
                match session.exit_code {
                    Some(code) => format!("exited({code})"),
                    None => "running".to_owned(),
                },
                session.activity.map_or("-", Activity::as_str).to_owned(),
                if session.viewer {
                    "viewer".to_owned()
                } else if session.attached {
                    "yes".to_owned()
                } else {
                    "no".to_owned()
                },
                session.pending.to_string(),
                session
                    .hold_reason
                    .clone()
                    .unwrap_or_else(|| "-".to_owned()),
                quota::cell(session.quota),
                format!("{}x{}", session.cols, session.rows),
            ]
        })
        .collect()
}
fn message_value_rows(messages: &[MessageInfo]) -> Vec<[String; 6]> {
    messages
        .iter()
        .map(|message| {
            [
                message.id.clone(),
                message.from.clone(),
                message.to.clone(),
                message.state.clone(),
                message
                    .detail
                    .clone()
                    .or_else(|| message.hold_reason.clone())
                    .or_else(|| match message.evidence.as_deref() {
                        Some("submission_observed") | Some("native_receipt") => {
                            message.evidence.clone()
                        }
                        _ => None,
                    })
                    .unwrap_or_else(|| "-".to_owned()),
                message.subject.clone(),
            ]
        })
        .collect()
}

pub(crate) fn column_widths<R: AsRef<[String]>>(headers: &[&str], rows: &[R]) -> Vec<usize> {
    let mut widths: Vec<usize> = headers.iter().map(|header| header.len()).collect();
    for row in rows {
        for (index, value) in row.as_ref().iter().enumerate() {
            widths[index] = widths[index].max(value.len());
        }
    }
    widths
}

fn format_table<R: AsRef<[String]>>(headers: &[&str], rows: &[R]) -> String {
    let widths = column_widths(headers, rows);
    let mut output = String::new();
    output.push_str(&format_row(headers, &widths));
    output.push('\n');
    for row in rows {
        output.push_str(&format_row(row.as_ref(), &widths));
        output.push('\n');
    }
    output
}

fn session_team_cells(sessions: &[SessionSummary]) -> Option<Vec<String>> {
    sessions
        .iter()
        .any(|session| session.team.is_some())
        .then(|| {
            sessions
                .iter()
                .map(|session| {
                    session.team.as_ref().map_or_else(
                        || "-".to_owned(),
                        |team| {
                            if team.private {
                                format!("{} (private)", team.name)
                            } else {
                                team.name.clone()
                            }
                        },
                    )
                })
                .collect()
        })
}

fn insert_team_column<'a>(
    headers: &'a [&'a str],
    mut rows: Vec<Vec<String>>,
    teams: Option<Vec<String>>,
) -> (Vec<&'a str>, Vec<Vec<String>>) {
    let Some(teams) = teams else {
        return (headers.to_vec(), rows);
    };
    let mut headers = headers.to_vec();
    headers.insert(2, "TEAM");
    for (row, team) in rows.iter_mut().zip(teams) {
        row.insert(2, team);
    }
    (headers, rows)
}

pub fn format_session_table(sessions: &[SessionSummary], details: bool) -> String {
    let rows: Vec<Vec<String>> = session_value_rows(sessions)
        .into_iter()
        .zip(sessions)
        .map(|(row, session)| {
            let [
                id,
                name,
                harness,
                state,
                activity,
                attached,
                pending,
                held,
                quota,
                size,
            ] = row;
            vec![
                id,
                name,
                harness,
                session_confinement_mode(session).to_owned(),
                state,
                session
                    .uptime_secs
                    .map(|seconds| display_duration(Duration::from_secs(seconds)))
                    .unwrap_or_else(|| "-".to_owned()),
                activity,
                session
                    .activity_secs
                    .map(|seconds| display_duration(Duration::from_secs(seconds)))
                    .unwrap_or_else(|| "-".to_owned()),
                attached,
                pending,
                held,
                quota,
                size,
            ]
        })
        .collect();
    let (headers, rows) = if details {
        let rows = rows
            .into_iter()
            .zip(sessions)
            .map(|(mut row, session)| {
                row.push(session.cwd.clone().unwrap_or_else(|| "-".to_owned()));
                row.push(session.argv.join(" "));
                row
            })
            .collect();
        insert_team_column(&DETAIL_HEADERS, rows, session_team_cells(sessions))
    } else {
        insert_team_column(&SESSION_HEADERS, rows, session_team_cells(sessions))
    };
    format_table(&headers, &rows)
}

pub fn format_messages_table(messages: &[MessageInfo]) -> String {
    format_table(&MESSAGE_HEADERS, &message_value_rows(messages))
}

/// The state lines `a2amx explain` prints for a session: one header line, then for a running session activity, readiness, needed input, hold, queue and the reset verdict.
pub fn format_explain(info: &ExplainInfo) -> String {
    let mut output = String::new();
    if let Some(code) = info.exit_code {
        output.push_str(&format!(
            "{} ({}): {}, exited (code {code})\n",
            info.id,
            info.address,
            info.harness.as_str(),
        ));
    } else {
        output.push_str(&format!(
            "{} ({}): {}, running\n",
            info.id,
            info.address,
            info.harness.as_str()
        ));
        let activity = info.activity.map(Activity::as_str).unwrap_or("unknown");
        let cause = info.activity_cause.as_deref().unwrap_or("unknown");
        output.push_str(&format!("activity: {activity} ({cause})\n"));
        let ready = match info.ready {
            Some(true) => "yes",
            Some(false) if info.starting => "no (starting)",
            Some(false) => "no",
            None => "unknown",
        };
        output.push_str(&format!("ready: {ready}\n"));
        output.push_str(&format!(
            "needs input: {}\n",
            info.needs_input.as_deref().unwrap_or("no")
        ));
        if let Some(hold) = info.hold.as_ref() {
            output.push_str("hold: ");
            output.push_str(&hold.reason);
            if let Some(explanation) = hold.explanation.as_deref() {
                output.push_str(" - ");
                output.push_str(explanation);
            }
            output.push('\n');
        } else {
            output.push_str("hold: none\n");
        }
        output.push_str(&format!("queued: {}\n", info.pending));
        if info.reset.allowed {
            output.push_str("reset: gate open\n");
        } else {
            output.push_str("reset: refused (");
            output.push_str(info.reset.code.as_deref().unwrap_or("unknown"));
            output.push_str("): ");
            output.push_str(info.reset.reason.as_deref().unwrap_or("unknown"));
            output.push('\n');
        }
    }
    output
}

pub(crate) fn format_row<T: AsRef<str>>(values: &[T], widths: &[usize]) -> String {
    let mut output = String::new();
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            output.push_str("  ");
        }
        let value = value.as_ref();
        output.push_str(value);
        if index + 1 < values.len() {
            for _ in value.len()..widths[index] {
                output.push(' ');
            }
        }
    }
    output
}
