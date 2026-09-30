//! Claude Code `UserPromptSubmit` hook adapter.

use std::io::{self, Read, Write};
use std::time::Duration;

use serde_json::Value;

use crate::client::Client;
use crate::mcp;
use crate::wire::{Request, Response};

const HOOK_TIMEOUT: Duration = Duration::from_secs(4);
const MAX_STDIN_BYTES: usize = 2 * 1024 * 1024;
const MAX_HOOK_PROMPT_BYTES: usize = 512 * 1024;

const STDIN_READ_FAILED: &str = "cannot read hook input";
const STDIN_TOO_LARGE: &str = "hook input is too large";
const INVALID_PAYLOAD: &str = "invalid hook payload";
const PROMPT_MISSING: &str = "hook payload has no prompt";
const PROMPT_TOO_LARGE: &str = "hook prompt is too large";
const CONFIGURATION_FAILED: &str = "daemon credentials are unavailable";
const REQUEST_TIMED_OUT: &str = "daemon request timed out";
const REQUEST_FAILED: &str = "daemon request failed";
const UNEXPECTED_RESPONSE: &str = "unexpected daemon response";
const STDOUT_WRITE_FAILED: &str = "cannot write hook decision";

/// Read the harness payload, report it to the daemon, and fail open on every
/// error. Hook stdout is part of the harness conversation, so it stays empty
/// unless the daemon explicitly blocks the prompt.
pub async fn run() -> anyhow::Result<()> {
    let input = match read_stdin().await {
        Ok(input) => input,
        Err(ReadError::TooLarge) => {
            note(STDIN_TOO_LARGE);
            return Ok(());
        }
        Err(ReadError::Failed) => {
            note(STDIN_READ_FAILED);
            return Ok(());
        }
    };

    let mut payload: Value = match serde_json::from_slice(&input) {
        Ok(payload) => payload,
        Err(_) => {
            note(INVALID_PAYLOAD);
            return Ok(());
        }
    };
    let Some(object) = payload.as_object_mut() else {
        note(INVALID_PAYLOAD);
        return Ok(());
    };
    if let Some(event) = object.get("hook_event_name") {
        if event.as_str() != Some("UserPromptSubmit") {
            return Ok(());
        }
    }
    let Some(prompt) = object.remove("prompt") else {
        note(PROMPT_MISSING);
        return Ok(());
    };
    let Value::String(prompt) = prompt else {
        note(PROMPT_MISSING);
        return Ok(());
    };
    // shortcut: larger prompts are allowed unexamined; envelopes are about 33 KiB.
    // Raise the cap if huge human prompts need this same safety net.
    if prompt.len() > MAX_HOOK_PROMPT_BYTES {
        note(PROMPT_TOO_LARGE);
        return Ok(());
    }

    let (address, token) = match mcp::configuration() {
        Ok(configuration) => configuration,
        Err(_) => {
            note(CONFIGURATION_FAILED);
            return Ok(());
        }
    };
    let response = tokio::time::timeout(HOOK_TIMEOUT, async move {
        let mut client = Client::connect_addr(address, &token).await?;
        client.request(Request::ReportPrompt { prompt }).await
    })
    .await;
    match response {
        Err(_) => note(REQUEST_TIMED_OUT),
        Ok(Err(_)) => note(REQUEST_FAILED),
        Ok(Ok(Response::PromptVerdict { verdict, reason })) => match verdict.as_str() {
            "allow" => {}
            "block" => {
                let written = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
                    let stdout = io::stdout();
                    let mut output = stdout.lock();
                    serde_json::to_writer(
                        &mut output,
                        &serde_json::json!({
                            "decision": "block", "reason": reason.unwrap_or_default(),
                        }),
                    )?;
                    output.write_all(b"\n")?;
                    output.flush()?;
                    Ok(())
                })
                .await;
                if !matches!(written, Ok(Ok(()))) {
                    note(STDOUT_WRITE_FAILED);
                }
            }
            _ => note(UNEXPECTED_RESPONSE),
        },
        Ok(Ok(_)) => note(UNEXPECTED_RESPONSE),
    }
    Ok(())
}

enum ReadError {
    TooLarge,
    Failed,
}

async fn read_stdin() -> Result<Vec<u8>, ReadError> {
    tokio::task::spawn_blocking(|| {
        let stdin = io::stdin();
        let stdin = stdin.lock();
        let mut input = Vec::new();
        let limit = u64::try_from(MAX_STDIN_BYTES + 1).map_err(|_| ReadError::Failed)?;
        stdin
            .take(limit)
            .read_to_end(&mut input)
            .map_err(|_| ReadError::Failed)?;
        if input.len() > MAX_STDIN_BYTES {
            return Err(ReadError::TooLarge);
        }
        Ok(input)
    })
    .await
    .map_err(|_| ReadError::Failed)?
}

fn note(reason: &str) {
    // Hook diagnostics are best effort: a broken stderr must not turn fail-open
    // handling into a non-zero hook exit.
    let _ = writeln!(io::stderr().lock(), "a2amx hook: {reason}");
}
