#![allow(clippy::expect_used, clippy::unwrap_used)]

#[path = "../common/mod.rs"]
mod common;

use a2amx::client::Client;
use a2amx::harness::{Deliver, Harness};
use a2amx::messaging::TeamScope;
use a2amx::wire::{Request, Response};
use common::pty::PtyHarness;
use std::path::Path;
use std::process::{Command, Output};
use std::time::{Duration, Instant};

fn scope(name: &str, private: bool, allow: &[&str]) -> TeamScope {
    scope_with_agents(name, private, allow, &[])
}

fn scope_with_agents(name: &str, private: bool, allow: &[&str], agents: &[&str]) -> TeamScope {
    TeamScope {
        name: name.into(),
        private,
        allow: allow.iter().map(|entry| (*entry).to_owned()).collect(),
        agents: agents.iter().map(|entry| (*entry).to_owned()).collect(),
    }
}

async fn session(
    admin: &mut Client,
    dir: &std::path::Path,
    name: &str,
    team: Option<TeamScope>,
    watch: &[&str],
    control_from: &[&str],
) -> (String, Client) {
    let (_, token, address) = common::new_team_agent(
        admin,
        dir,
        Some(name),
        Harness::Generic,
        Deliver::Hold,
        team,
        watch,
        control_from,
    )
    .await;
    let client = Client::connect_addr(address, &token).await.unwrap();
    let id = loop {
        let response = admin.request(Request::List).await.unwrap();
        if let Response::Sessions { sessions } = response
            && let Some(session) = sessions
                .into_iter()
                .find(|session| session.name.as_deref() == Some(name))
        {
            break session.id;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    };
    (id, client)
}

mod team_up_remote;
mod team_validation;
mod team_visibility;
