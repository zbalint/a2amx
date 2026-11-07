//! Claude Code status-line adapter and quota reporter.

use std::io::{self, Read, Write};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::client::Client;
use crate::mcp;
use crate::quota;
use crate::wire::{QuotaWindow, Request, Response};

const MAX_STDIN_BYTES: usize = 256 * 1024;
const REPORT_TIMEOUT: Duration = Duration::from_millis(500);

/// Read Claude's status-line payload, report valid quota, and render its short line.
pub async fn run(report: bool) -> anyhow::Result<()> {
    let input = match tokio::task::spawn_blocking(read_stdin).await? {
        Ok(input) => input,
        Err(ReadError::TooLarge) => {
            note("status-line input is too large");
            if !report {
                write_output(String::new()).await;
            }
            return Ok(());
        }
        Err(ReadError::Failed) => {
            note("cannot read status-line input");
            if !report {
                write_output(String::new()).await;
            }
            return Ok(());
        }
    };
    let payload: Value = match serde_json::from_slice(&input) {
        Ok(payload) => payload,
        Err(_) => {
            note("invalid status-line payload");
            if !report {
                write_output(String::new()).await;
            }
            return Ok(());
        }
    };
    let Some(limits) = quota::from_claude_rate_limits(&payload, now_unix()) else {
        if !report {
            write_output(String::new()).await;
        }
        return Ok(());
    };
    let line = format_line(&limits);
    report_quota(limits).await;
    if !report {
        write_output(line).await;
    }
    Ok(())
}

fn read_stdin() -> Result<Vec<u8>, ReadError> {
    let mut input = Vec::with_capacity(MAX_STDIN_BYTES + 1);
    let read = io::stdin()
        .take((MAX_STDIN_BYTES + 1) as u64)
        .read_to_end(&mut input)
        .map_err(|_| ReadError::Failed)?;
    if read > MAX_STDIN_BYTES {
        return Err(ReadError::TooLarge);
    }
    Ok(input)
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

fn format_line(limits: &quota::ClaudeRateLimits) -> String {
    [
        limits
            .five_hour
            .map(|window| format!("5h {}% left", window.left)),
        limits
            .weekly
            .map(|window| format!("7d {}% left", window.left)),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ")
}

async fn report_quota(limits: quota::ClaudeRateLimits) {
    let (address, token) = match mcp::configuration() {
        Ok(configuration) => configuration,
        Err(_) => {
            note("daemon credentials are unavailable");
            return;
        }
    };
    let request = Request::ReportQuota {
        five_hour: limits.five_hour.map(|window| QuotaWindow {
            left: window.left,
            resets_at: window.resets_at,
        }),
        weekly: limits.weekly.map(|window| QuotaWindow {
            left: window.left,
            resets_at: window.resets_at,
        }),
    };
    let result = tokio::time::timeout(REPORT_TIMEOUT, async move {
        let mut client = Client::connect_addr(address, &token).await?;
        client.request(request).await
    })
    .await;
    if !matches!(result, Ok(Ok(Response::Ok))) {
        note("daemon quota report failed");
    }
}

async fn write_output(line: String) {
    let _ = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let stdout = io::stdout();
        let mut output = stdout.lock();
        output.write_all(line.as_bytes())?;
        output.write_all(b"\n")?;
        output.flush()?;
        Ok(())
    })
    .await;
}

fn note(message: &str) {
    eprintln!("a2amx statusline: {message}");
}

enum ReadError {
    Failed,
    TooLarge,
}
