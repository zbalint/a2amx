use super::*;

#[test]
fn summary_activity_serializes_literals_and_old_payloads_remain_unknown() {
    let activities = [
        (Activity::Idle, "idle"),
        (Activity::Working, "working"),
        (Activity::Busy, "busy"),
    ];
    for (activity, expected) in activities {
        assert_eq!(serde_json::to_value(activity).unwrap(), expected);
    }
    let session_json = serde_json::json!({
        "id": "s1", "argv": ["sh"], "cols": 80, "rows": 24,
        "exit_code": null, "attached": false,
    });
    let agent_json = serde_json::json!({
        "address": "agent-plan@host-a", "state": "running",
        "attached": false, "harness": "generic",
    });
    for old in [session_json, agent_json] {
        for activity in [None, Some("idle"), Some("working"), Some("busy")] {
            let mut payload = old.clone();
            if let Some(activity) = activity {
                payload["activity"] = activity.into();
            }
            let encoded = if payload.get("id").is_some() {
                let summary: SessionSummary = serde_json::from_value(payload.clone()).unwrap();
                serde_json::to_value(summary).unwrap()
            } else {
                let summary: a2amx::wire::AgentSummary =
                    serde_json::from_value(payload.clone()).unwrap();
                serde_json::to_value(summary).unwrap()
            };
            assert_eq!(encoded, payload);
        }
    }
}
#[test]
fn quota_info_serializes_percent_left_field_names() {
    use a2amx::wire::QuotaInfo;

    for (quota, expected) in [
        (
            QuotaInfo {
                five_hour_left: Some(15),
                weekly_left: None,
                limit_reached: false,
            },
            serde_json::json!({"five_hour_left": 15}),
        ),
        (
            QuotaInfo {
                five_hour_left: Some(15),
                weekly_left: Some(0),
                limit_reached: true,
            },
            serde_json::json!({
                "five_hour_left": 15,
                "weekly_left": 0,
                "limit_reached": true,
            }),
        ),
    ] {
        assert_eq!(serde_json::to_value(quota).unwrap(), expected);
    }
}

#[test]
fn response_and_session_summary_shapes_remain_unchanged() {
    let response = Response::Sessions {
        sessions: vec![SessionSummary {
            id: "s1".to_owned(),
            argv: vec!["cat".to_owned()],
            cols: 80,
            rows: 24,
            exit_code: None,
            attached: false,
            viewer: false,
            name: None,
            address: String::new(),
            pending: 0,
            held: false,
            hold_reason: None,
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
    let json = serde_json::to_string(&response).unwrap_or_default();
    assert_eq!(
        json,
        r#"{"type":"sessions","sessions":[{"id":"s1","argv":["cat"],"cols":80,"rows":24,"exit_code":null,"attached":false}]}"#
    );
    assert_eq!(serde_json::from_str::<Response>(&json).ok(), Some(response));
}

#[test]
fn session_summary_durations_round_trip_when_present() {
    let response = Response::Sessions {
        sessions: vec![SessionSummary {
            id: "s2".to_owned(),
            argv: vec!["sh".to_owned()],
            cols: 80,
            rows: 24,
            exit_code: None,
            attached: false,
            viewer: false,
            name: None,
            address: String::new(),
            pending: 0,
            held: false,
            hold_reason: None,
            quota: None,
            harness: Default::default(),
            confine: None,
            team: None,
            cwd: None,
            activity: Some(Activity::Working),
            uptime_secs: Some(5),
            activity_secs: Some(3),
            starting: false,
        }],
    };
    let json = serde_json::to_string(&response).unwrap_or_default();
    assert!(json.contains(r#""uptime_secs":5,"activity_secs":3"#));
    assert_eq!(serde_json::from_str::<Response>(&json).ok(), Some(response));
}
#[test]
fn session_summary_starting_is_optional_and_round_trips() {
    let mut summary = SessionSummary {
        id: "s-starting".to_owned(),
        argv: vec!["claude".to_owned()],
        cols: 80,
        rows: 24,
        exit_code: None,
        attached: false,
        viewer: false,
        name: None,
        address: String::new(),
        pending: 0,
        held: false,
        hold_reason: None,
        quota: None,
        harness: Default::default(),
        confine: None,
        team: None,
        cwd: None,
        activity: None,
        uptime_secs: None,
        activity_secs: None,
        starting: true,
    };
    let encoded = serde_json::to_value(&summary).unwrap();
    assert_eq!(encoded["starting"], true);
    assert_eq!(
        serde_json::from_value::<SessionSummary>(encoded).unwrap(),
        summary
    );

    summary.starting = false;
    let encoded = serde_json::to_value(&summary).unwrap();
    assert!(encoded.get("starting").is_none());

    let old: SessionSummary = serde_json::from_value(serde_json::json!({
        "id": "s-old",
        "argv": ["cat"],
        "cols": 80,
        "rows": 24,
        "exit_code": null,
        "attached": false,
    }))
    .unwrap();
    assert!(!old.starting);
}

#[test]
fn viewer_summary_fields_round_trip_and_default_for_old_clients() {
    let session = SessionSummary {
        id: "s-viewer".into(),
        argv: vec!["cat".into()],
        cols: 80,
        rows: 24,
        exit_code: None,
        attached: true,
        viewer: true,
        name: None,
        address: String::new(),
        pending: 0,
        held: false,
        hold_reason: None,
        quota: None,
        harness: Default::default(),
        confine: None,
        team: None,
        cwd: None,
        activity: None,
        uptime_secs: None,
        activity_secs: None,
        starting: false,
    };
    let encoded = serde_json::to_value(&session).unwrap();
    assert_eq!(encoded["viewer"], true);
    assert_eq!(
        serde_json::from_value::<SessionSummary>(encoded).unwrap(),
        session
    );
    let old_session: SessionSummary = serde_json::from_value(serde_json::json!({
        "id": "s-old",
        "argv": ["cat"],
        "cols": 80,
        "rows": 24,
        "exit_code": null,
        "attached": true,
    }))
    .unwrap();
    assert!(!old_session.viewer);

    let agent = AgentSummary {
        address: "agent-viewer@host-a".into(),
        state: "running".into(),
        attached: true,
        viewer: true,
        team: None,
        you: false,
        quota: None,
        harness: Default::default(),
        cwd: None,
        activity: None,
        confine: None,
    };
    let encoded = serde_json::to_value(&agent).unwrap();
    assert_eq!(encoded["viewer"], true);
    assert_eq!(
        serde_json::from_value::<AgentSummary>(encoded).unwrap(),
        agent
    );
    let old_agent: AgentSummary = serde_json::from_value(serde_json::json!({
        "address": "agent-old@host-a",
        "state": "running",
        "attached": true,
    }))
    .unwrap();
    assert!(!old_agent.viewer);
}
