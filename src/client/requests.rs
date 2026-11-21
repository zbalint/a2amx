//! Typed control requests: one method per request, each with the reply semantics its callers need.

use anyhow::anyhow;

use crate::wire::{ExplainInfo, MessageInfo, Request, Response, SessionSummary};

use super::Client;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedLeaseCode {
    pub code: String,
    pub agents: Vec<String>,
    pub valid_secs: u64,
}

fn unexpected(response: Response) -> anyhow::Error {
    anyhow!("unexpected daemon response: {response:?}")
}

impl Client {
    pub async fn sessions(&mut self) -> anyhow::Result<Vec<SessionSummary>> {
        match self.request(Request::List).await? {
            Response::Sessions { sessions } => Ok(sessions),
            Response::Error { message } => Err(anyhow!(message)),
            other => Err(unexpected(other)),
        }
    }

    pub async fn kill(&mut self, session: String, now: bool) -> anyhow::Result<()> {
        match self.request(Request::Kill { session, now }).await? {
            Response::Ok => Ok(()),
            Response::Error { message } => Err(anyhow!(message)),
            other => Err(unexpected(other)),
        }
    }

    pub async fn reset(&mut self, session: &str) -> anyhow::Result<u32> {
        match self
            .request(Request::Reset {
                session: session.to_owned(),
            })
            .await?
        {
            Response::Reset { steps } => Ok(steps),
            Response::Failed { code, message } => Err(anyhow!("{code}: {message}")),
            Response::Error { message } => Err(anyhow!(message)),
            other => Err(unexpected(other)),
        }
    }

    pub async fn shutdown(&mut self, now: bool) -> anyhow::Result<()> {
        match self.request(Request::Shutdown { now }).await? {
            Response::Ok => Ok(()),
            Response::Error { message } => Err(anyhow!(message)),
            other => Err(unexpected(other)),
        }
    }

    pub async fn version(&mut self) -> anyhow::Result<Option<String>> {
        match self.request(Request::Version).await? {
            Response::Version { version } => Ok(Some(version)),
            Response::Error { .. } => Ok(None),
            other => Err(unexpected(other)),
        }
    }

    pub async fn explain(&mut self, session: String) -> anyhow::Result<ExplainInfo> {
        match self.request(Request::Explain { session }).await? {
            Response::Explain { info } => Ok(info),
            Response::Error { message } => Err(anyhow!(message)),
            other => Err(unexpected(other)),
        }
    }

    pub async fn screen(&mut self, session: String) -> anyhow::Result<Vec<String>> {
        match self.request(Request::Screen { session }).await? {
            Response::Screen { lines } => Ok(lines),
            Response::Error { message } => Err(anyhow!(message)),
            other => Err(unexpected(other)),
        }
    }

    pub async fn messages(
        &mut self,
        session: Option<String>,
        state: Option<String>,
    ) -> anyhow::Result<Vec<MessageInfo>> {
        match self
            .request(Request::ListMessages { session, state })
            .await?
        {
            Response::Messages { messages } => Ok(messages),
            Response::Error { message } | Response::Failed { message, .. } => Err(anyhow!(message)),
            other => Err(unexpected(other)),
        }
    }

    pub async fn cancel_message(&mut self, id: String) -> anyhow::Result<()> {
        match self.request(Request::CancelMessage { id }).await? {
            Response::Ok => Ok(()),
            Response::Error { message } | Response::Failed { message, .. } => Err(anyhow!(message)),
            other => Err(unexpected(other)),
        }
    }

    pub async fn issue_lease_code(&mut self, team: String) -> anyhow::Result<IssuedLeaseCode> {
        match self.request(Request::IssueLeaseCode { team }).await? {
            Response::LeaseCode {
                code,
                agents,
                valid_secs,
            } => Ok(IssuedLeaseCode {
                code,
                agents,
                valid_secs,
            }),
            Response::Error { message } => Err(anyhow!(message)),
            other => Err(unexpected(other)),
        }
    }

    pub async fn create_session(&mut self, request: Request) -> anyhow::Result<String> {
        let confined = matches!(request, Request::Confine { .. });
        match self.request(request).await? {
            Response::Created { session } => Ok(session),
            Response::Error { message }
                if confined && message.contains("invalid control request") =>
            {
                Err(anyhow!(
                    "the running daemon does not support --confine; restart it"
                ))
            }
            Response::Error { message } => Err(anyhow!(message)),
            other => Err(unexpected(other)),
        }
    }
}
