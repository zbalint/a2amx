use super::*;

#[test]
fn request_json_uses_exact_tagged_shapes() {
    let request = Request::NewSession {
        argv: vec!["sh".to_owned(), "-c".to_owned(), "cat".to_owned()],
        cols: 80,
        rows: 24,
        cwd: Some("/tmp/work".to_owned()),
        env: vec![("TERM".to_owned(), "xterm-256color".to_owned())],
        reset: Vec::new(),
        control_from: Vec::new(),
        watch: Vec::new(),
        name: None,
        harness: a2amx::harness::Harness::Generic,
        deliver: None,
        heartbeat: None,
        activity: None,
        team: None,
        role: None,
        remote: None,
    };
    let json = serde_json::to_string(&request).unwrap_or_default();
    assert_eq!(
        json,
        r#"{"type":"new_session","argv":["sh","-c","cat"],"cols":80,"rows":24,"cwd":"/tmp/work","env":[["TERM","xterm-256color"]]}"#
    );
    assert_eq!(serde_json::from_str::<Request>(&json).ok(), Some(request));

    let hello = serde_json::to_string(&Request::Hello {
        token: "secret".to_owned(),
    })
    .unwrap_or_default();
    assert_eq!(hello, r#"{"type":"hello","token":"secret"}"#);
    assert_eq!(
        serde_json::to_string(&Request::Attach {
            session: "s1".to_owned(),
            force: true,
            cols: 120,
            rows: 40,
            status: false,
            top_rows: 0,
            viewer: false,
        })
        .unwrap_or_default(),
        r#"{"type":"attach","session":"s1","force":true,"cols":120,"rows":40}"#
    );
    let screen = Request::Screen {
        session: "s1".to_owned(),
    };
    let screen_json = serde_json::to_string(&screen).unwrap_or_default();
    assert_eq!(screen_json, r#"{"type":"screen","session":"s1"}"#);
    assert_eq!(
        serde_json::from_str::<Request>(&screen_json).ok(),
        Some(screen)
    );
    let quota = Request::ReportQuota {
        five_hour: Some(a2amx::wire::QuotaWindow {
            left: 77,
            resets_at: 2000,
        }),
        weekly: None,
    };
    let quota_json = serde_json::to_string(&quota).unwrap_or_default();
    assert_eq!(
        quota_json,
        r#"{"type":"report_quota","five_hour":{"left":77,"resets_at":2000},"weekly":null}"#
    );
    assert_eq!(
        serde_json::from_str::<Request>(&quota_json).ok(),
        Some(quota)
    );
}

#[test]
fn explain_wire_round_trips_and_omits_absent_options() {
    let request = Request::Explain {
        session: "s1".into(),
    };
    let request_json = serde_json::to_string(&request).unwrap_or_default();
    assert_eq!(request_json, r#"{"type":"explain","session":"s1"}"#);
    assert_eq!(
        serde_json::from_str::<Request>(&request_json).ok(),
        Some(request)
    );

    let populated = Response::Explain {
        info: ExplainInfo {
            id: "s1".into(),
            address: "worker@host-a".into(),
            harness: a2amx::harness::Harness::Generic,
            exit_code: Some(7),
            ready: Some(true),
            starting: true,
            activity: Some(Activity::Busy),
            activity_cause: Some("held: human_draft".into()),
            needs_input: Some("input?".into()),
            hold: Some(HoldInfo {
                reason: "human_draft".into(),
                explanation: Some("draft explanation".into()),
            }),
            pending: 2,
            reset: ResetVerdict {
                allowed: false,
                code: Some("draft_present".into()),
                reason: Some("hold human_draft".into()),
            },
        },
    };
    let populated_json = serde_json::to_string(&populated).unwrap_or_default();
    assert_eq!(
        serde_json::from_str::<Response>(&populated_json).ok(),
        Some(populated)
    );

    let absent = Response::Explain {
        info: ExplainInfo {
            id: "s2".into(),
            address: "plain@host-a".into(),
            harness: a2amx::harness::Harness::Generic,
            exit_code: None,
            ready: None,
            starting: false,
            activity: None,
            activity_cause: None,
            needs_input: None,
            hold: None,
            pending: 0,
            reset: ResetVerdict {
                allowed: true,
                code: None,
                reason: None,
            },
        },
    };
    let absent_json = serde_json::to_value(&absent).unwrap();
    let info = absent_json["info"].as_object().unwrap();
    for key in [
        "exit_code",
        "ready",
        "activity",
        "activity_cause",
        "needs_input",
        "hold",
    ] {
        assert!(!info.contains_key(key), "{key} should be omitted");
    }
    let reset = info["reset"].as_object().unwrap();
    assert!(!reset.contains_key("code"));
    assert!(!reset.contains_key("reason"));
    assert_eq!(
        serde_json::from_value::<Response>(absent_json).ok(),
        Some(absent)
    );
}

#[test]
fn activity_watch_round_trips_on_new_session() {
    let request = Request::NewSession {
        argv: vec!["sh".into()],
        cols: 40,
        rows: 10,
        cwd: None,
        env: Vec::new(),
        reset: Vec::new(),
        control_from: Vec::new(),
        watch: vec!["peer".into()],
        activity: Some(a2amx::wire::ActivityWatch {
            working_after: Some(120),
            idle_after: Some(30),
        }),
        heartbeat: None,
        name: Some("watcher".into()),
        harness: a2amx::harness::Harness::Generic,
        deliver: None,
        team: None,
        role: None,
        remote: None,
    };
    let value = serde_json::to_value(&request).unwrap();
    assert_eq!(value["activity"]["working_after"], 120);
    assert_eq!(value["activity"]["idle_after"], 30);
    assert_eq!(serde_json::from_value::<Request>(value).unwrap(), request);
}

#[test]
fn activity_watch_validation_uses_literal_bounds_and_errors() {
    use a2amx::wire::ActivityWatch;

    let no_watch = ActivityWatch {
        working_after: Some(1),
        idle_after: None,
    };
    assert_eq!(
        no_watch.validate(false).unwrap_err().to_string(),
        "activity needs a non-empty watch"
    );
    let no_duration = ActivityWatch {
        working_after: None,
        idle_after: None,
    };
    assert_eq!(
        no_duration.validate(true).unwrap_err().to_string(),
        "activity needs at least one duration"
    );
    for (watch, expected) in [
        (
            ActivityWatch {
                working_after: Some(0),
                idle_after: None,
            },
            "invalid activity working_after \"0s\": expected a number followed by s, m or h, from 1s to 24h",
        ),
        (
            ActivityWatch {
                working_after: Some(86_401),
                idle_after: None,
            },
            "invalid activity working_after \"86401s\": expected a number followed by s, m or h, from 1s to 24h",
        ),
        (
            ActivityWatch {
                working_after: None,
                idle_after: Some(86_401),
            },
            "invalid activity idle_after \"86401s\": expected a number followed by s, m or h, from 1s to 24h",
        ),
    ] {
        assert_eq!(watch.validate(true).unwrap_err().to_string(), expected);
    }
    for watch in [
        ActivityWatch {
            working_after: Some(1),
            idle_after: None,
        },
        ActivityWatch {
            working_after: Some(86_400),
            idle_after: Some(1),
        },
        ActivityWatch {
            working_after: None,
            idle_after: Some(86_400),
        },
    ] {
        assert!(watch.validate(true).is_ok());
    }
}

#[test]
fn version_request_and_response_use_exact_shapes() {
    let json = serde_json::to_string(&Request::Version).unwrap();
    assert_eq!(json, r#"{"type":"version"}"#);
    assert_eq!(
        serde_json::from_str::<Request>(&json).unwrap(),
        Request::Version
    );

    let response = Response::Version {
        version: "ab12cd3-dirty".into(),
    };
    let json = serde_json::to_string(&response).unwrap();
    assert_eq!(json, r#"{"type":"version","version":"ab12cd3-dirty"}"#);
    assert_eq!(serde_json::from_str::<Response>(&json).unwrap(), response);
}

#[test]
fn lease_requests_and_response_use_exact_shapes() {
    let issue = Request::IssueLeaseCode {
        team: "team-a".into(),
    };
    let issue_json = serde_json::to_string(&issue).unwrap();
    assert_eq!(issue_json, r#"{"type":"issue_lease_code","team":"team-a"}"#);
    assert_eq!(serde_json::from_str::<Request>(&issue_json).unwrap(), issue);

    let redeem = Request::RedeemLeaseCode {
        code: "0123456789ab".into(),
    };
    let redeem_json = serde_json::to_string(&redeem).unwrap();
    assert_eq!(
        redeem_json,
        r#"{"type":"redeem_lease_code","code":"0123456789ab"}"#
    );
    assert_eq!(
        serde_json::from_str::<Request>(&redeem_json).unwrap(),
        redeem
    );

    let response = Response::LeaseCode {
        code: "0123456789ab".into(),
        agents: vec!["team-a-architect (both)".into()],
        valid_secs: 600,
    };
    let response_json = serde_json::to_string(&response).unwrap();
    assert_eq!(
        response_json,
        r#"{"type":"lease_code","code":"0123456789ab","agents":["team-a-architect (both)"],"valid_secs":600}"#
    );
    assert_eq!(
        serde_json::from_str::<Response>(&response_json).unwrap(),
        response
    );
}

#[test]
fn reset_request_and_response_use_exact_shapes() {
    let request = Request::Reset {
        session: "s2".to_owned(),
    };
    let json = serde_json::to_string(&request).unwrap_or_default();
    assert_eq!(json, r#"{"type":"reset","session":"s2"}"#);
    assert_eq!(serde_json::from_str::<Request>(&json).ok(), Some(request));

    let response = Response::Reset { steps: 2 };
    let json = serde_json::to_string(&response).unwrap_or_default();
    assert_eq!(json, r#"{"type":"reset","steps":2}"#);
    assert_eq!(serde_json::from_str::<Response>(&json).ok(), Some(response));
}

#[test]
fn screen_response_json_uses_exact_shape() {
    let response = Response::Screen {
        lines: vec!["first".to_owned(), "second".to_owned()],
    };
    let json = serde_json::to_string(&response).unwrap_or_default();
    assert_eq!(json, r#"{"type":"screen","lines":["first","second"]}"#);
    assert_eq!(serde_json::from_str::<Response>(&json).ok(), Some(response));
}

#[test]
fn attach_top_rows_preserves_old_wire_and_acknowledges_new_wire() {
    let old = r#"{"type":"attach","session":"s1","force":false,"cols":80,"rows":23}"#;
    let request: Request = serde_json::from_str(old).unwrap();
    assert!(matches!(request, Request::Attach { top_rows: 0, .. }));
    assert_eq!(serde_json::to_string(&request).unwrap(), old);
    let response = Response::AttachedTop { top_rows: 1 };
    let json = serde_json::to_string(&response).unwrap();
    assert_eq!(json, r#"{"type":"attached_top","top_rows":1}"#);
    assert_eq!(serde_json::from_str::<Response>(&json).unwrap(), response);
    assert_eq!(
        serde_json::from_str::<Response>(r#"{"type":"attached"}"#).unwrap(),
        Response::Attached
    );
}

#[test]
fn report_quota_request_round_trips() {
    let request = Request::ReportQuota {
        five_hour: Some(a2amx::wire::QuotaWindow {
            left: 77,
            resets_at: 2000,
        }),
        weekly: Some(a2amx::wire::QuotaWindow {
            left: 59,
            resets_at: 2000,
        }),
    };
    let encoded = serde_json::to_string(&request).unwrap();
    assert_eq!(
        encoded,
        r#"{"type":"report_quota","five_hour":{"left":77,"resets_at":2000},"weekly":{"left":59,"resets_at":2000}}"#
    );
    assert_eq!(serde_json::from_str::<Request>(&encoded).unwrap(), request);
}
