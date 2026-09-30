use a2amx::emulator::Scroll;
use a2amx::wire::{
    ClientFrame, FrameDecoder, MAX_FRAME_LEN, Request, Response, ServerFrame, SessionSummary,
    encode_frame,
};

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
        })
        .unwrap_or_default(),
        r#"{"type":"attach","session":"s1","force":true,"cols":120,"rows":40}"#
    );
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
