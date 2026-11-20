//! Asking the daemon for the session list and resolving a user's session reference.

use std::path::Path;

use anyhow::anyhow;

use crate::client::Client;
use crate::wire::{Request, Response, SessionSummary};

pub async fn request_sessions(client: &mut Client) -> anyhow::Result<Vec<SessionSummary>> {
    match client.request(Request::List).await? {
        Response::Sessions { sessions } => Ok(sessions),
        Response::Error { message } => Err(anyhow!(message)),
        other => Err(anyhow!("unexpected daemon response: {other:?}")),
    }
}
/// A name resolves to its id; other references remain for the daemon to judge.
pub fn resolve_reference(sessions: &[SessionSummary], reference: &str) -> String {
    sessions
        .iter()
        .find(|session| session.name.as_deref() == Some(reference))
        .map_or_else(|| reference.to_owned(), |session| session.id.clone())
}

pub async fn resolve_session(home: &Path, reference: &str) -> anyhow::Result<String> {
    let mut client = Client::connect(home).await?;
    Ok(resolve_reference(
        &request_sessions(&mut client).await?,
        reference,
    ))
}

pub(crate) async fn session_in_team(home: &Path, reference: &str) -> anyhow::Result<bool> {
    let mut client = Client::connect(home).await?;
    let sessions = request_sessions(&mut client).await?;
    let id = resolve_reference(&sessions, reference);
    Ok(sessions
        .iter()
        .find(|session| session.id == id)
        .is_some_and(|session| session.team.is_some()))
}
