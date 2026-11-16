use super::*;

#[test]
fn additive_visibility_fields_round_trip() {
    let accepted = Response::Accepted {
        id: "m_2".into(),
        recipient_hold: Some("deliver_hold".into()),
        recipient_quota: None,
    };
    let accepted_json = serde_json::json!({
        "type": "accepted",
        "id": "m_2",
        "recipient_hold": "deliver_hold",
    });
    assert_eq!(serde_json::to_value(&accepted).unwrap(), accepted_json);
    assert_eq!(
        serde_json::from_value::<Response>(accepted_json).unwrap(),
        accepted
    );

    let sessions = Response::Sessions {
        sessions: vec![SessionSummary {
            id: "s2".into(),
            argv: vec!["sh".into()],
            cols: 120,
            rows: 40,
            exit_code: None,
            attached: false,
            viewer: false,
            name: Some("agent-review".into()),
            address: "agent-review@host-a".into(),
            pending: 1,
            held: true,
            hold_reason: Some("human_draft".into()),
            quota: None,
            harness: Default::default(),
            confine: None,
            team: None,
            cwd: None,
            activity: None,
            uptime_secs: None,
            activity_secs: None,
            starting: false,
        }],
    };
    let sessions_json = serde_json::json!({
        "type": "sessions",
        "sessions": [{
            "id": "s2",
            "argv": ["sh"],
            "cols": 120,
            "rows": 40,
            "exit_code": null,
            "attached": false,
            "name": "agent-review",
            "address": "agent-review@host-a",
            "pending": 1,
            "held": true,
            "hold_reason": "human_draft",
        }],
    });
    assert_eq!(serde_json::to_value(&sessions).unwrap(), sessions_json);
    assert_eq!(
        serde_json::from_value::<Response>(sessions_json).unwrap(),
        sessions
    );

    let status = Response::Status {
        message: MessageInfo {
            id: "m_2".into(),
            from: "agent-plan@host-a".into(),
            to: "agent-review@host-a".into(),
            subject: "Parser issue".into(),
            state: "pending".into(),
            detail: None,
            hold_reason: Some("deliver_hold".into()),
            evidence: Some("write_complete".into()),
            hold_explanation: Some(
                "The recipient session only holds messages; a person must deliver by hand or restart it with automatic delivery.".into(),
            ),
            accepted_at: Some(2_000_000_000),
            updated_at: Some(2_000_000_001),
        },
    };
    let status_json = serde_json::json!({
        "type": "status",
        "message": {
            "id": "m_2",
            "from": "agent-plan@host-a",
            "to": "agent-review@host-a",
            "subject": "Parser issue",
            "state": "pending",
            "detail": null,
            "hold_reason": "deliver_hold",
            "evidence": "write_complete",
            "hold_explanation": "The recipient session only holds messages; a person must deliver by hand or restart it with automatic delivery.",
            "accepted_at": 2_000_000_000i64,
            "updated_at": 2_000_000_001i64,
        },
    });
    assert_eq!(serde_json::to_value(&status).unwrap(), status_json);
    assert_eq!(
        serde_json::from_value::<Response>(status_json).unwrap(),
        status
    );
}
#[test]
fn confinement_visibility_fields_are_backward_compatible() {
    let old_session: SessionSummary = serde_json::from_value(serde_json::json!({
        "id": "s-old",
        "argv": ["sh"],
        "cols": 80,
        "rows": 24,
        "exit_code": null,
        "attached": false
    }))
    .expect("old session payload");
    assert_eq!(old_session.confine, None);

    let old_agent: AgentSummary = serde_json::from_value(serde_json::json!({
        "address": "agent-old@host-a",
        "state": "running",
        "attached": false
    }))
    .expect("old agent payload");
    assert_eq!(old_agent.confine, None);

    let old_status: StatusInfo =
        serde_json::from_value(serde_json::json!({"address": "agent-old@host-a", "pending": 0}))
            .expect("old status payload");
    assert!(!old_status.confine_auto);

    let old_confine_json = serde_json::json!({
        "auto": true,
        "grants": ["cache", "ssh"],
        "write": ["~/.cache"],
        "reasons": [{
            "grant": "cache",
            "reason": "always added by auto"
        }]
    });
    let old_confine: ConfineInfo =
        serde_json::from_value(old_confine_json).expect("old confinement payload");
    assert!(old_confine.hide.is_empty());
    assert!(old_confine.unhide.is_empty());
    assert!(!old_confine.no_default_hides);

    let confine = ConfineInfo {
        auto: true,
        grants: vec!["cache".into(), "ssh".into()],
        write: vec!["~/.cache".into()],
        reasons: vec![GrantReason {
            grant: "cache".into(),
            reason: "always added by auto".into(),
        }],
        hide: vec!["~/.secret".into()],
        unhide: vec!["~/.ssh".into()],
        no_default_hides: true,
    };
    let encoded = serde_json::to_value(&confine).expect("confinement payload");
    assert_eq!(
        serde_json::from_value::<ConfineInfo>(encoded).unwrap(),
        confine
    );

    let defaults = ConfineInfo {
        auto: false,
        grants: Vec::new(),
        write: Vec::new(),
        reasons: Vec::new(),
        hide: Vec::new(),
        unhide: Vec::new(),
        no_default_hides: false,
    };
    let encoded_defaults = serde_json::to_value(defaults).expect("default confinement payload");
    assert!(!encoded_defaults.as_object().unwrap().contains_key("hide"));
    assert!(!encoded_defaults.as_object().unwrap().contains_key("unhide"));
    assert!(
        !encoded_defaults
            .as_object()
            .unwrap()
            .contains_key("no_default_hides")
    );
}

#[test]
fn status_frames_and_attach_opt_in_preserve_wire_compatibility() {
    use a2amx::harness::Harness;
    use a2amx::wire::{QuotaInfo, StatusInfo};
    let info = StatusInfo {
        address: "agent-plan@host-a".into(),
        pending: 2,
        hold: Some("human_draft".into()),
        harness: Some(Harness::Claude),
        activity: Some(Activity::Working),
        confined: false,
        confine_auto: false,
        starting: true,
        quota: Some(QuotaInfo {
            five_hour_left: Some(15),
            weekly_left: None,
            limit_reached: false,
        }),
        team: None,
        role: None,
        members: vec![],
        remote: Some(a2amx::wire::RemoteStatus {
            lease: a2amx::wire::RemoteLease::Active,
            gateway: true,
        }),
    };
    let frame = ServerFrame::Status(info);
    let encoded = frame.encode();
    assert_eq!(encoded[0], 0x04);
    let payload: serde_json::Value = serde_json::from_slice(&encoded[1..]).unwrap();
    assert_eq!(
        payload.get("remote").unwrap(),
        &serde_json::json!({"lease": "active", "gateway": true})
    );
    assert_eq!(ServerFrame::decode(&encoded).unwrap(), frame);
    let old_payload = [vec![0x04], br#"{"address":"a","pending":0}"#.to_vec()].concat();
    let ServerFrame::Status(old_info) = ServerFrame::decode(&old_payload).unwrap() else {
        panic!("old status payload");
    };
    assert_eq!(old_info.address, "a");
    assert_eq!(old_info.pending, 0);
    assert_eq!(old_info.hold, None);
    assert_eq!(old_info.harness, None);
    assert_eq!(old_info.activity, None);
    assert!(!old_info.starting);
    assert_eq!(old_info.quota, None);
    assert_eq!(old_info.remote, None);
    let old_member: TeamMember = serde_json::from_value(
        serde_json::json!({"id":"s0","name":"old","activity":"idle","exited":false}),
    )
    .unwrap();
    assert!(!old_member.held);
    assert!(!old_member.starting);
    assert!(ServerFrame::decode(&[0x04, b'x']).is_err());
    assert!(
        ServerFrame::decode(
            &[vec![0x04], vec![b' '; a2amx::wire::MAX_SERVER_DATA_LEN + 1]].concat()
        )
        .is_err()
    );
    let request = Request::Attach {
        session: "s1".into(),
        force: false,
        cols: 80,
        rows: 23,
        status: true,
        top_rows: 0,
        viewer: false,
    };
    assert_eq!(
        serde_json::to_string(&request).unwrap(),
        r#"{"type":"attach","session":"s1","force":false,"cols":80,"rows":23,"status":true}"#
    );
    assert_eq!(
        serde_json::from_str::<Request>(
            r#"{"type":"attach","session":"s1","force":false,"cols":80,"rows":23}"#
        )
        .unwrap(),
        Request::Attach {
            session: "s1".into(),
            force: false,
            cols: 80,
            rows: 23,
            status: false,
            top_rows: 0,
            viewer: false,
        }
    );
}

#[test]
fn status_members_default_omit_and_round_trip() {
    let old: a2amx::wire::StatusInfo =
        serde_json::from_value(serde_json::json!({"address": "a", "pending": 0})).unwrap();
    assert!(old.members.is_empty());
    assert!(!old.starting);

    let empty = serde_json::to_value(&old).unwrap();
    assert!(empty.get("members").is_none());
    assert!(empty.get("starting").is_none());
    assert!(empty.get("confine_auto").is_none());
    assert!(empty.get("confined").is_none());

    let old_member: TeamMember = serde_json::from_value(
        serde_json::json!({"id":"old","name":"old","activity":"idle","exited":false}),
    )
    .unwrap();
    let old_member_json = serde_json::to_value(old_member).unwrap();
    assert!(old_member_json.get("held").is_none());
    assert!(old_member_json.get("starting").is_none());

    let status = a2amx::wire::StatusInfo {
        address: "a".into(),
        pending: 0,
        hold: None,
        harness: None,
        activity: None,
        quota: None,
        confined: false,
        confine_auto: false,
        starting: true,
        team: None,
        role: None,
        members: vec![
            TeamMember {
                id: "s1".into(),
                name: "alpha".into(),
                activity: Some(Activity::Idle),
                exited: false,
                held: true,
                starting: true,
            },
            TeamMember {
                id: "s2".into(),
                name: "beta".into(),
                activity: None,
                exited: true,
                held: false,
                starting: false,
            },
        ],
        remote: None,
    };
    let encoded = serde_json::to_value(&status).unwrap();
    assert!(encoded.get("remote").is_none());
    assert_eq!(
        serde_json::from_value::<a2amx::wire::StatusInfo>(encoded).unwrap(),
        status
    );
}

#[test]
fn team_fields_round_trip_and_old_new_session_defaults_are_compatible() {
    use a2amx::messaging::TeamScope;
    let request = Request::NewSession {
        argv: vec!["sh".into()],
        cols: 40,
        rows: 10,
        cwd: None,
        env: Vec::new(),
        reset: Vec::new(),
        control_from: Vec::new(),
        watch: Vec::new(),
        name: Some("demo-worker".into()),
        harness: a2amx::harness::Harness::Generic,
        deliver: None,
        heartbeat: None,
        activity: None,
        team: Some(TeamScope {
            name: "demo".into(),
            private: true,
            allow: vec!["other".into()],
            agents: Vec::new(),
        }),
        role: Some("worker".into()),
        remote: None,
    };
    let value = serde_json::to_value(&request).unwrap();
    assert_eq!(value["team"]["name"], "demo");
    assert_eq!(value["team"]["private"], true);
    assert_eq!(value["role"], "worker");
    assert_eq!(serde_json::from_value::<Request>(value).unwrap(), request);
    let scope: TeamScope = serde_json::from_value(serde_json::json!({
        "name": "demo",
        "private": true,
        "allow": [],
    }))
    .unwrap();
    assert!(scope.agents.is_empty());
    let value = serde_json::to_value(scope).unwrap();
    assert!(!value.as_object().unwrap().contains_key("agents"));
    let old = serde_json::json!({
        "type": "new_session",
        "argv": ["sh"],
        "cols": 40,
        "rows": 10,
        "cwd": null,
        "env": [],
        "name": "worker",
        "harness": "generic"
    });
    let Request::NewSession { team, role, .. } = serde_json::from_value(old).unwrap() else {
        panic!("old request tag");
    };
    assert_eq!(team, None);
    assert_eq!(role, None);
}

#[test]
fn confine_request_round_trips_and_defaults_missing_fields() {
    let request = Request::Confine {
        confine: a2amx::confine::ConfineRequest {
            write: vec!["workspace".into()],
            hide: vec!["secret".into()],
            unhide: vec![],
            no_default_hides: true,
            grants: vec![],
            auto: false,
            reasons: vec![],
            keep_env: vec!["GITHUB_TOKEN".into()],
            team_file: Some("/workspace/team.toml".into()),
        },
        request: Box::new(Request::NewSession {
            argv: vec!["sh".into()],
            cols: 40,
            rows: 10,
            cwd: Some("/workspace".into()),
            env: vec![],
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            activity: None,
            heartbeat: None,
            name: None,
            harness: a2amx::harness::Harness::Generic,
            deliver: None,
            team: None,
            role: None,
            remote: None,
        }),
    };
    let encoded = serde_json::to_value(&request).unwrap();
    assert_eq!(
        serde_json::from_value::<Request>(encoded.clone()).unwrap(),
        request
    );
    assert_eq!(
        encoded["confine"]["team_file"],
        serde_json::json!("/workspace/team.toml")
    );
    assert_eq!(
        encoded["confine"]["keep_env"],
        serde_json::json!(["GITHUB_TOKEN"])
    );
    let mut old = encoded;
    old["confine"]
        .as_object_mut()
        .expect("confine object")
        .remove("team_file");
    old["confine"]
        .as_object_mut()
        .expect("confine object")
        .remove("keep_env");
    let Request::Confine { confine, .. } = serde_json::from_value(old).unwrap() else {
        panic!("confine request tag");
    };
    assert_eq!(confine.team_file, None);
    assert!(confine.keep_env.is_empty());
    let absent: a2amx::confine::ConfineRequest =
        serde_json::from_value(serde_json::json!({})).unwrap();
    assert_eq!(absent, a2amx::confine::ConfineRequest::default());

    let mut old_allow = serde_json::to_value(&request).unwrap();
    let confine = old_allow["confine"]
        .as_object_mut()
        .expect("confine object");
    confine.remove("grants");
    confine.insert("allow_containers".into(), serde_json::json!(true));
    let Request::Confine { confine, .. } = serde_json::from_value(old_allow).unwrap() else {
        panic!("old confine request tag");
    };
    assert!(confine.grants.is_empty());
}
