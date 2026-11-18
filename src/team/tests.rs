use super::*;
use std::collections::BTreeSet;

use crate::confine::ConfineRequest;
use crate::grant::Grant;
use crate::wire::GrantReason;

fn session(name: &str, auto: bool, confine: Option<ConfineRequest>) -> TeamSession {
    TeamSession {
        name: name.to_owned(),
        command: vec!["sh".to_owned()],
        cwd: None,
        attach: false,
        reset: None,
        control_from: Vec::new(),
        watch: Vec::new(),
        activity: None,
        heartbeat: None,
        role: None,
        remote: None,
        allow_agents: Vec::new(),
        confine,
        auto,
        team: None,
    }
}

fn request(grants: &[&str], reasons: &[(&str, &str)]) -> ConfineRequest {
    ConfineRequest {
        grants: grants.iter().map(|grant| (*grant).to_owned()).collect(),
        reasons: reasons
            .iter()
            .map(|(grant, reason)| GrantReason {
                grant: (*grant).to_owned(),
                reason: (*reason).to_owned(),
            })
            .collect(),
        ..ConfineRequest::default()
    }
}

#[test]
fn auto_confine_requires_auto_and_confine() {
    let without_confine = session("without-confine", true, None);
    assert!(without_confine.auto_confine().is_none());

    let without_auto = session(
        "without-auto",
        false,
        Some(request(&["cache"], &[("cache", "detected")])),
    );
    assert!(without_auto.auto_confine().is_none());

    let mut both = session(
        "both",
        true,
        Some(request(&["cache"], &[("cache", "detected")])),
    );
    assert!(both.auto_confine().is_some());
    assert!(both.auto_confine_mut().is_some());
}

#[test]
fn new_auto_grants_deduplicates_across_sessions_with_first_reason() {
    let actions = vec![
        Action::Start(Box::new(session(
            "first",
            true,
            Some(request(&["cache"], &[("cache", "first reason")])),
        ))),
        Action::Start(Box::new(session(
            "second",
            true,
            Some(request(&["cache"], &[("cache", "later reason")])),
        ))),
    ];

    assert_eq!(
        new_auto_grants(&actions, &BTreeSet::new()),
        vec![("cache".to_owned(), "first reason".to_owned())]
    );
}

#[test]
fn new_auto_grants_skips_accepted_grants() {
    let actions = vec![Action::Start(Box::new(session(
        "worker",
        true,
        Some(request(
            &["cache", "cargo"],
            &[("cache", "cache reason"), ("cargo", "cargo reason")],
        )),
    )))];
    let accepted = BTreeSet::from(["cache".to_owned()]);

    assert_eq!(
        new_auto_grants(&actions, &accepted),
        vec![("cargo".to_owned(), "cargo reason".to_owned())]
    );
}

#[test]
fn new_auto_grants_skips_non_auto_and_unconfined_sessions() {
    let actions = vec![
        Action::Start(Box::new(session(
            "manual",
            false,
            Some(request(&["cache"], &[("cache", "manual reason")])),
        ))),
        Action::Start(Box::new(session("plain", true, None))),
        Action::AlreadyRunning {
            name: "running".to_owned(),
            id: "id".to_owned(),
        },
    ];

    assert!(new_auto_grants(&actions, &BTreeSet::new()).is_empty());
}

fn grant(name: &'static str, escape: &'static str) -> Grant {
    Grant {
        name,
        exposes: "test exposure",
        escape: Some(escape),
        team_default_ok: true,
        sockets: &[],
        env: &[],
        own_config: false,
        unhides: &[],
        detect: &[],
        create: &[],
        stand_ins: &[],
    }
}

#[test]
fn escape_notices_only_render_explicit_grants_per_started_session() {
    let actions = vec![
        Action::Start(Box::new(session(
            "worker",
            true,
            Some(request(
                &["manual", "auto"],
                &[("auto", "detected automatically")],
            )),
        ))),
        Action::Start(Box::new(session(
            "quiet",
            true,
            Some(request(&["auto"], &[("auto", "detected automatically")])),
        ))),
        Action::AlreadyRunning {
            name: "running".to_owned(),
            id: "id".to_owned(),
        },
    ];
    let rows = [
        grant("manual", "manual escape"),
        grant("auto", "auto escape"),
    ];

    assert_eq!(
        escape_notices(&actions, &rows),
        vec!["session worker: grant manual can escape: manual escape\n".to_owned()]
    );
}

#[test]
fn escape_notices_skip_grants_without_escape_text() {
    let actions = vec![Action::Start(Box::new(session(
        "worker",
        false,
        Some(request(&["manual"], &[])),
    )))];
    let rows = [Grant {
        name: "manual",
        exposes: "test exposure",
        escape: None,
        team_default_ok: true,
        sockets: &[],
        env: &[],
        own_config: false,
        unhides: &[],
        detect: &[],
        create: &[],
        stand_ins: &[],
    }];

    assert!(escape_notices(&actions, &rows).is_empty());
}
