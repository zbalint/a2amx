use a2amx::emulator::Scroll;
use a2amx::wire::{
    Activity, ClientFrame, FrameDecoder, MAX_FRAME_LEN, MessageInfo, Request, Response,
    ServerFrame, SessionSummary, encode_frame,
};

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
fn length_prefixed_frame_round_trips() {
    let payload = b"hello\0world";
    let encoded = encode_frame(payload).ok();
    assert!(encoded.is_some());
    let encoded = encoded.unwrap_or_default();
    assert_eq!(&encoded[..4], &[0, 0, 0, 11]);

    let mut decoder = FrameDecoder::default();
    assert_eq!(decoder.push(&encoded).ok(), Some(vec![payload.to_vec()]));
}

#[test]
fn frame_decoder_accepts_every_split_boundary() {
    let encoded = encode_frame(b"split me").unwrap_or_default();
    for split in 0..=encoded.len() {
        let mut decoder = FrameDecoder::default();
        let mut frames = decoder.push(&encoded[..split]).unwrap_or_default();
        frames.extend(decoder.push(&encoded[split..]).unwrap_or_default());
        assert_eq!(frames, vec![b"split me".to_vec()], "split at {split}");
    }
}

#[test]
fn frame_decoder_rejects_oversized_declared_length_before_payload() {
    let oversized = u32::try_from(MAX_FRAME_LEN + 1)
        .unwrap_or(u32::MAX)
        .to_be_bytes();
    let mut decoder = FrameDecoder::default();
    let error = decoder.push(&oversized).err();
    assert!(error.is_some());
    let message = error.map_or_else(String::new, |error| error.to_string());
    assert!(
        message.contains("maximum") || message.contains("large"),
        "{message}"
    );
}

#[test]
fn frame_length_limit_accepts_the_boundary_and_rejects_one_byte_over() {
    assert!(encode_frame(&vec![0; MAX_FRAME_LEN]).is_ok());
    assert!(encode_frame(&vec![0; MAX_FRAME_LEN + 1]).is_err());
}

#[test]
fn frame_decoder_bounds_partial_payload_buffer_and_handles_following_frame() {
    let first = encode_frame(&vec![b'x'; MAX_FRAME_LEN]).unwrap_or_default();
    let second = encode_frame(b"next").unwrap_or_default();
    let mut decoder = FrameDecoder::default();
    let mut frames = decoder
        .push(&first[..MAX_FRAME_LEN / 2])
        .unwrap_or_default();
    assert!(frames.is_empty());
    frames.extend(
        decoder
            .push(&first[MAX_FRAME_LEN / 2..])
            .unwrap_or_default(),
    );
    frames.extend(decoder.push(&second).unwrap_or_default());
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0].len(), MAX_FRAME_LEN);
    assert_eq!(frames[1], b"next".to_vec());
}

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
fn response_and_session_summary_shapes_remain_unchanged() {
    let response = Response::Sessions {
        sessions: vec![SessionSummary {
            id: "s1".to_owned(),
            argv: vec!["cat".to_owned()],
            cols: 80,
            rows: 24,
            exit_code: None,
            attached: false,
            name: None,
            address: String::new(),
            pending: 0,
            held: false,
            hold_reason: None,
            quota: None,
            harness: Default::default(),
            cwd: None,
            activity: None,
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
fn screen_response_json_uses_exact_shape() {
    let response = Response::Screen {
        lines: vec!["first".to_owned(), "second".to_owned()],
    };
    let json = serde_json::to_string(&response).unwrap_or_default();
    assert_eq!(json, r#"{"type":"screen","lines":["first","second"]}"#);
    assert_eq!(serde_json::from_str::<Response>(&json).ok(), Some(response));
}

#[test]
fn every_server_stream_frame_round_trips() {
    let frames = [
        ServerFrame::Data(vec![0, 1, 2, 255]),
        ServerFrame::Exit(-17),
        ServerFrame::Detached("taken over".to_owned()),
    ];
    for frame in frames {
        let payload = frame.encode();
        assert_eq!(ServerFrame::decode(&payload).ok(), Some(frame));
    }
}

#[test]
fn every_client_stream_frame_round_trips() {
    let frames = [
        ClientFrame::Input(vec![0, 1, 2, 255]),
        ClientFrame::Resize {
            cols: 120,
            rows: 40,
        },
        ClientFrame::Detach,
        ClientFrame::Redraw,
        ClientFrame::Scroll(Scroll::LineUp),
        ClientFrame::Scroll(Scroll::LineDown),
        ClientFrame::Scroll(Scroll::PageUp),
        ClientFrame::Scroll(Scroll::PageDown),
        ClientFrame::Scroll(Scroll::Top),
        ClientFrame::Scroll(Scroll::Bottom),
    ];
    for frame in frames {
        let payload = frame.encode();
        assert_eq!(ClientFrame::decode(&payload).ok(), Some(frame));
    }
}

#[test]
fn stream_frames_have_the_specified_byte_layout() {
    assert_eq!(ServerFrame::Data(vec![1, 2]).encode(), vec![0x01, 1, 2]);
    assert_eq!(
        ServerFrame::Exit(-1).encode(),
        vec![0x02, 0xff, 0xff, 0xff, 0xff]
    );
    assert_eq!(
        ServerFrame::Detached("é".to_owned()).encode(),
        vec![0x03, 0xc3, 0xa9]
    );
    assert_eq!(ClientFrame::Input(vec![1, 2]).encode(), vec![0x01, 1, 2]);
    assert_eq!(
        ClientFrame::Resize {
            cols: 0x0102,
            rows: 0x0304
        }
        .encode(),
        vec![0x02, 1, 2, 3, 4]
    );
    assert_eq!(ClientFrame::Detach.encode(), vec![0x03]);
    assert_eq!(ClientFrame::Redraw.encode(), vec![0x04]);
    assert_eq!(ClientFrame::Scroll(Scroll::Bottom).encode(), vec![0x05, 5]);
}

#[test]
fn stream_decoders_reject_malformed_payloads() {
    for payload in [
        vec![],
        vec![0x7f],
        vec![0x02],
        vec![0x02, 0, 0, 0],
        vec![0x03, 0xff],
    ] {
        assert!(
            ServerFrame::decode(&payload).is_err(),
            "server payload {payload:?}"
        );
    }
    for payload in [
        vec![],
        vec![0x7f],
        vec![0x02],
        vec![0x02, 0, 1, 2],
        vec![0x03, 0],
        vec![0x04, 0],
        vec![0x05],
        vec![0x05, 6],
    ] {
        assert!(
            ClientFrame::decode(&payload).is_err(),
            "client payload {payload:?}"
        );
    }
}

#[test]
fn stream_decoders_enforce_input_and_data_limits() {
    let input_at_limit = vec![b'i'; 8 * 1024];
    assert!(ClientFrame::decode(&[vec![0x01], input_at_limit].concat()).is_ok());
    let input = vec![b'i'; 8 * 1024 + 1];
    assert!(ClientFrame::decode(&[vec![0x01], input].concat()).is_err());

    let data_at_limit = vec![b'd'; 64 * 1024];
    assert!(ServerFrame::decode(&[vec![0x01], data_at_limit].concat()).is_ok());
    let data = vec![b'd'; 64 * 1024 + 1];
    assert!(ServerFrame::decode(&[vec![0x01], data].concat()).is_err());
}

#[test]
fn detached_reason_must_be_utf8() {
    assert!(ServerFrame::decode(&[0x03, 0xff]).is_err());
}

#[test]
fn release_has_an_empty_body_and_rejects_extra_bytes() {
    assert_eq!(ClientFrame::Release.encode(), vec![0x06]);
    assert_eq!(
        ClientFrame::decode(&[0x06]).ok(),
        Some(ClientFrame::Release)
    );
    assert!(ClientFrame::decode(&[0x06, 0]).is_err());
}

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
                quota: None,
                harness: Default::default(),
                cwd: None,
                activity: None,
            }],
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
            quota: None,
            harness: Harness::Omp,
            cwd: Some("/tmp/agent".into()),
            activity: None,
        }],
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
    let Response::Agents { agents } = serde_json::from_value(old_agents).unwrap() else {
        panic!("old agents payload did not deserialize");
    };
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
            name: Some("agent-review".into()),
            address: "agent-review@host-a".into(),
            pending: 1,
            held: true,
            hold_reason: Some("human_draft".into()),
            quota: None,
            harness: Default::default(),
            cwd: None,
            activity: None,
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
fn status_frames_and_attach_opt_in_preserve_wire_compatibility() {
    use a2amx::harness::Harness;
    use a2amx::wire::{QuotaInfo, StatusInfo};
    let info = StatusInfo {
        address: "agent-plan@host-a".into(),
        pending: 2,
        hold: Some("human_draft".into()),
        harness: Some(Harness::Claude),
        activity: Some(Activity::Working),
        quota: Some(QuotaInfo {
            five_hour: Some(15),
            weekly: None,
            limit_reached: false,
        }),
    };
    let frame = ServerFrame::Status(info);
    let encoded = frame.encode();
    assert_eq!(encoded[0], 0x04);
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
    assert_eq!(old_info.quota, None);
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
            status: false
        }
    );
}
