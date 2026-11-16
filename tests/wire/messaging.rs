use super::*;

#[test]
fn messaging_control_variants_round_trip() {
    use a2amx::wire::{AgentSummary, MessageInfo};
    let requests = [
        Request::SendMessage {
            to: "agent-review@host-a".into(),
            subject: "Parser issue".into(),
            message: "I found the regression in parser.py.".into(),
        },
        Request::ListAgents,
        Request::MessageStatus { id: "m_1".into() },
        Request::ListMessages {
            session: Some("s2".into()),
            state: Some("pending".into()),
        },
        Request::CancelMessage { id: "m_1".into() },
        Request::NewSession {
            argv: vec!["sh".into()],
            cols: 40,
            rows: 10,
            cwd: None,
            env: vec![],
            reset: Vec::new(),
            control_from: Vec::new(),
            watch: Vec::new(),
            name: Some("agent-review".into()),
            harness: a2amx::harness::Harness::Claude,
            deliver: Some(a2amx::harness::Deliver::Auto),
            heartbeat: None,
            activity: None,
            team: None,
            role: None,
            remote: None,
        },
    ];
    let request_json = [
        serde_json::json!({"type":"send_message","to":"agent-review@host-a","subject":"Parser issue","message":"I found the regression in parser.py."}),
        serde_json::json!({"type":"list_agents"}),
        serde_json::json!({"type":"message_status","id":"m_1"}),
        serde_json::json!({"type":"list_messages","session":"s2","state":"pending"}),
        serde_json::json!({"type":"cancel_message","id":"m_1"}),
        serde_json::json!({"type":"new_session","argv":["sh"],"cols":40,"rows":10,"cwd":null,"env":[],"name":"agent-review","harness":"claude","deliver":"auto"}),
    ];
    for (request, expected) in requests.into_iter().zip(request_json) {
        assert_eq!(serde_json::to_value(&request).unwrap(), expected);
        let bytes = serde_json::to_vec(&request).unwrap();
        assert_eq!(serde_json::from_slice::<Request>(&bytes).unwrap(), request);
    }
    let message = MessageInfo {
        id: "m_1".into(),
        from: "agent-plan@host-a".into(),
        to: "agent-review@host-a".into(),
        subject: "Parser issue".into(),
        state: "pending".into(),
        detail: None,
        hold_reason: Some("human_draft".into()),
        evidence: None,
        hold_explanation: None,
        accepted_at: None,
        updated_at: None,
    };
    let responses = [
        Response::Accepted {
            id: "m_1".into(),
            recipient_hold: None,
            recipient_quota: None,
        },
        Response::Failed {
            code: "queue_full".into(),
            message: "recipient queue is full".into(),
        },
        Response::Agents {
            agents: vec![AgentSummary {
                address: "agent-plan@host-a".into(),
                state: "running".into(),
                attached: false,
                viewer: false,
                team: None,
                you: false,
                quota: None,
                harness: Default::default(),
                confine: None,
                cwd: None,
                activity: None,
            }],
            owner: None,
        },
        Response::Status {
            message: message.clone(),
        },
        Response::Messages {
            messages: vec![message],
        },
    ];
    let response_json = [
        serde_json::json!({"type":"accepted","id":"m_1"}),
        serde_json::json!({"type":"failed","code":"queue_full","message":"recipient queue is full"}),
        serde_json::json!({"type":"agents","agents":[{"address":"agent-plan@host-a","state":"running","attached":false,"harness":"generic"}]}),
        serde_json::json!({"type":"status","message":{"id":"m_1","from":"agent-plan@host-a","to":"agent-review@host-a","subject":"Parser issue","state":"pending","detail":null,"hold_reason":"human_draft"}}),
        serde_json::json!({"type":"messages","messages":[{"id":"m_1","from":"agent-plan@host-a","to":"agent-review@host-a","subject":"Parser issue","state":"pending","detail":null,"hold_reason":"human_draft"}]}),
    ];
    for (response, expected) in responses.into_iter().zip(response_json) {
        assert_eq!(serde_json::to_value(&response).unwrap(), expected);
        let bytes = serde_json::to_vec(&response).unwrap();
        assert_eq!(
            serde_json::from_slice::<Response>(&bytes).unwrap(),
            response
        );
    }
    use a2amx::harness::Harness;
    let with_cwd = Response::Agents {
        agents: vec![AgentSummary {
            address: "agent-omp@host-a".into(),
            state: "running".into(),
            attached: false,
            viewer: false,
            team: None,
            you: false,
            quota: None,
            harness: Harness::Omp,
            confine: None,
            cwd: Some("/tmp/agent".into()),
            activity: None,
        }],
        owner: None,
    };
    let with_cwd_json = serde_json::json!({"type":"agents","agents":[{"address":"agent-omp@host-a","state":"running","attached":false,"harness":"omp","cwd":"/tmp/agent"}]});
    assert_eq!(serde_json::to_value(&with_cwd).unwrap(), with_cwd_json);
    assert_eq!(
        serde_json::from_value::<Response>(with_cwd_json).unwrap(),
        with_cwd
    );
    let old_agents = serde_json::json!({
        "type": "agents",
        "agents": [{
            "address": "agent-plan@host-a",
            "state": "running",
            "attached": false,
        }],
    });
    let Response::Agents { agents, owner } = serde_json::from_value(old_agents).unwrap() else {
        panic!("old agents payload did not deserialize");
    };
    assert_eq!(owner, None);
    assert_eq!(agents[0].harness, Harness::Generic);
    assert_eq!(agents[0].cwd, None);
}

#[test]
fn prompt_reports_verdicts_and_optional_evidence_have_exact_json() {
    use a2amx::wire::MessageInfo;
    let request = Request::ReportPrompt { prompt: "x".into() };
    let encoded = serde_json::to_string(&request).unwrap();
    assert_eq!(encoded, r#"{"type":"report_prompt","prompt":"x"}"#);
    assert_eq!(serde_json::from_str::<Request>(&encoded).unwrap(), request);
    for (response, expected) in [
        (
            Response::PromptVerdict {
                verdict: "allow".into(),
                reason: None,
            },
            r#"{"type":"prompt_verdict","verdict":"allow"}"#,
        ),
        (
            Response::PromptVerdict {
                verdict: "block".into(),
                reason: Some("r".into()),
            },
            r#"{"type":"prompt_verdict","verdict":"block","reason":"r"}"#,
        ),
    ] {
        let encoded = serde_json::to_string(&response).unwrap();
        assert_eq!(encoded, expected);
        assert_eq!(
            serde_json::from_str::<Response>(&encoded).unwrap(),
            response
        );
    }
    let old = r#"{"id":"m_1","from":"agent-plan@host-a","to":"agent-review@host-a","subject":"Parser issue","state":"submitted","detail":null,"hold_reason":null}"#;
    let mut message: MessageInfo = serde_json::from_str(old).unwrap();
    assert_eq!(message.evidence, None);
    assert_eq!(message.hold_explanation, None);
    assert_eq!(message.accepted_at, None);
    assert_eq!(message.updated_at, None);
    assert_eq!(serde_json::to_string(&message).unwrap(), old);
    message.evidence = Some("submission_observed".into());
    let observed = r#"{"id":"m_1","from":"agent-plan@host-a","to":"agent-review@host-a","subject":"Parser issue","state":"submitted","detail":null,"hold_reason":null,"evidence":"submission_observed"}"#;
    assert_eq!(serde_json::to_string(&message).unwrap(), observed);
    assert_eq!(
        serde_json::from_str::<MessageInfo>(observed).unwrap(),
        message
    );
}

#[test]
fn human_outbound_requests_and_response_use_exact_shapes() {
    let next = Request::HumanOutboundNext;
    let next_json = serde_json::to_string(&next).unwrap();
    assert_eq!(next_json, r#"{"type":"human_outbound_next"}"#);
    assert_eq!(serde_json::from_str::<Request>(&next_json).unwrap(), next);

    let done = Request::HumanOutboundDone {
        id: "m_7".into(),
        sent: true,
    };
    let done_json = serde_json::to_string(&done).unwrap();
    assert_eq!(
        done_json,
        r#"{"type":"human_outbound_done","id":"m_7","sent":true}"#
    );
    assert_eq!(serde_json::from_str::<Request>(&done_json).unwrap(), done);

    let response = Response::HumanOutbound {
        message: Some(a2amx::wire::HumanOutboundMessage {
            id: "m_7".into(),
            from: "agent-plan@host-a".into(),
            subject: "Parser issue".into(),
            body: "I found the regression.".into(),
        }),
    };
    let response_json = serde_json::to_string(&response).unwrap();
    assert_eq!(
        response_json,
        r#"{"type":"human_outbound","message":{"id":"m_7","from":"agent-plan@host-a","subject":"Parser issue","body":"I found the regression."}}"#
    );
    assert_eq!(
        serde_json::from_str::<Response>(&response_json).unwrap(),
        response
    );
    let empty = Response::HumanOutbound { message: None };
    assert_eq!(
        serde_json::to_string(&empty).unwrap(),
        r#"{"type":"human_outbound","message":null}"#
    );
}

#[test]
fn gateway_agents_request_and_response_use_exact_shapes() {
    let request = Request::GatewayAgents;
    let request_json = serde_json::to_string(&request).unwrap();
    assert_eq!(request_json, r#"{"type":"gateway_agents"}"#);
    assert_eq!(
        serde_json::from_str::<Request>(&request_json).unwrap(),
        request
    );

    let response = Response::GatewayAgents {
        agents: vec![a2amx::wire::GatewayAgent {
            address: "agent-plan@host-a".into(),
            name: "agent-plan".into(),
            team: "team-a".into(),
            mode: "both".into(),
        }],
    };
    let response_json = serde_json::to_string(&response).unwrap();
    assert_eq!(
        response_json,
        r#"{"type":"gateway_agents","agents":[{"address":"agent-plan@host-a","name":"agent-plan","team":"team-a","mode":"both"}]}"#
    );
    assert_eq!(
        serde_json::from_str::<Response>(&response_json).unwrap(),
        response
    );
}
