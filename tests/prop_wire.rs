use a2amx::emulator::Scroll;
use a2amx::wire::{
    ClientFrame, FrameDecoder, MAX_FRAME_LEN, MAX_INPUT_LEN, MAX_SERVER_DATA_LEN, ServerFrame,
    encode_frame,
};
use proptest::prelude::*;

fn payloads() -> impl Strategy<Value = Vec<Vec<u8>>> {
    prop::collection::vec(prop::collection::vec(any::<u8>(), 0..=2048), 0..=8)
}

fn detached_reasons() -> impl Strategy<Value = String> {
    prop::collection::vec(any::<char>(), 0..=256).prop_map(|chars| chars.into_iter().collect())
}

fn server_frames() -> impl Strategy<Value = ServerFrame> {
    prop_oneof![
        prop::collection::vec(any::<u8>(), 0..=2048).prop_map(ServerFrame::Data),
        any::<i32>().prop_map(ServerFrame::Exit),
        detached_reasons().prop_map(ServerFrame::Detached),
    ]
}

fn scrolls() -> impl Strategy<Value = Scroll> {
    prop_oneof![
        Just(Scroll::LineUp),
        Just(Scroll::LineDown),
        Just(Scroll::PageUp),
        Just(Scroll::PageDown),
        Just(Scroll::Top),
        Just(Scroll::Bottom),
    ]
}

fn client_frames() -> impl Strategy<Value = ClientFrame> {
    prop_oneof![
        prop::collection::vec(any::<u8>(), 0..=2048).prop_map(ClientFrame::Input),
        (any::<u16>(), any::<u16>()).prop_map(|(cols, rows)| ClientFrame::Resize { cols, rows }),
        Just(ClientFrame::Detach),
        Just(ClientFrame::Redraw),
        scrolls().prop_map(ClientFrame::Scroll),
        Just(ClientFrame::Release),
    ]
}

fn valid_scroll_bytes() -> Vec<u8> {
    [
        Scroll::LineUp,
        Scroll::LineDown,
        Scroll::PageUp,
        Scroll::PageDown,
        Scroll::Top,
        Scroll::Bottom,
    ]
    .into_iter()
    .map(|scroll| ClientFrame::Scroll(scroll).encode()[1])
    .collect()
}

fn malformed_client_frames() -> impl Strategy<Value = Vec<u8>> {
    let resize = (0usize..=8)
        .prop_filter("resize body length is not four", |length| *length != 4)
        .prop_map(|length| {
            let mut frame = vec![0x02];
            frame.extend(std::iter::repeat_n(0, length));
            frame
        });
    let empty_body =
        (prop::sample::select(vec![0x03, 0x04, 0x06]), 1usize..=8).prop_map(|(tag, length)| {
            let mut frame = vec![tag];
            frame.extend(std::iter::repeat_n(0, length));
            frame
        });
    let scroll_length = (0usize..=8)
        .prop_filter("scroll body length is not one", |length| *length != 1)
        .prop_map(|length| {
            let mut frame = vec![0x05];
            frame.extend(std::iter::repeat_n(0, length));
            frame
        });
    let valid = valid_scroll_bytes();
    let invalid_scroll = (0u8..=u8::MAX)
        .prop_filter("scroll byte is not a valid scroll variant", move |byte| {
            !valid.contains(byte)
        })
        .prop_map(|byte| vec![0x05, byte]);
    prop_oneof![resize, empty_body, scroll_length, invalid_scroll]
}

fn malformed_server_frames() -> impl Strategy<Value = Vec<u8>> {
    let exit = (0usize..=8)
        .prop_filter("exit body length is not four", |length| *length != 4)
        .prop_map(|length| {
            let mut frame = vec![0x02];
            frame.extend(std::iter::repeat_n(0, length));
            frame
        });
    let detached = prop::collection::vec(any::<u8>(), 0..=8).prop_map(|mut body| {
        body.insert(0, 0xff);
        let mut frame = vec![0x03];
        frame.extend(body);
        frame
    });
    prop_oneof![exit, detached]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn frames_survive_any_read_boundaries(
        payloads in payloads(),
        cuts in prop::collection::vec(any::<u16>(), 0..=32),
    ) {
        let mut stream = Vec::new();
        for payload in &payloads {
            stream.extend(encode_frame(payload).unwrap_or_else(|error| panic!("generated payload failed: {error}")));
        }

        let mut boundaries = cuts
            .into_iter()
            .map(|cut| usize::from(cut) % (stream.len() + 1))
            .collect::<Vec<_>>();
        boundaries.sort_unstable();
        let mut decoder = FrameDecoder::default();
        let mut got = Vec::new();
        let mut start = 0;
        for boundary in boundaries.into_iter().chain(std::iter::once(stream.len())) {
            got.extend(decoder.push(&stream[start..boundary]).unwrap_or_else(|error| panic!("valid stream failed: {error}")));
            start = boundary;
        }
        prop_assert_eq!(got, payloads);
    }

    #[test]
    fn encoded_frame_is_big_endian_length_then_payload(payload in prop::collection::vec(any::<u8>(), 0..=4096)) {
        let encoded = encode_frame(&payload).unwrap_or_else(|error| panic!("generated payload failed: {error}"));
        prop_assert_eq!(&encoded[..4], &(payload.len() as u32).to_be_bytes());
        prop_assert_eq!(&encoded[4..], payload.as_slice());
    }

    #[test]
    fn oversized_declared_length_errors_before_payload(
        declared in (MAX_FRAME_LEN as u64 + 1..=u64::from(u32::MAX))
    ) {
        let mut decoder = FrameDecoder::default();
        let header = (declared as u32).to_be_bytes();
        prop_assert!(decoder.push(&header).is_err());
    }

    #[test]
    fn server_frames_round_trip(frame in server_frames()) {
        prop_assert_eq!(
            ServerFrame::decode(&frame.encode()).unwrap_or_else(|error| panic!("encoded server frame failed: {error}")),
            frame
        );
    }

    #[test]
    fn client_frames_round_trip(frame in client_frames()) {
        prop_assert_eq!(
            ClientFrame::decode(&frame.encode()).unwrap_or_else(|error| panic!("encoded client frame failed: {error}")),
            frame
        );
    }

    #[test]
    fn decode_never_panics_and_rejects_unknown_tags(bytes in prop::collection::vec(any::<u8>(), 0..=64)) {
        let client = ClientFrame::decode(&bytes);
        let server = ServerFrame::decode(&bytes);
        if bytes.is_empty() {
            prop_assert!(client.is_err());
            prop_assert!(server.is_err());
        } else {
            if !matches!(bytes[0], 0x01..=0x06) {
                prop_assert!(client.is_err());
            }
            if !matches!(bytes[0], 0x01..=0x04) {
                prop_assert!(server.is_err());
            }
        }
    }
    #[test]
    fn malformed_known_tag_bodies_are_rejected(
        client in malformed_client_frames(),
        server in malformed_server_frames(),
    ) {
        prop_assert!(ClientFrame::decode(&client).is_err());
        prop_assert!(ServerFrame::decode(&server).is_err());
    }

}
#[test]
fn decoder_accepts_a_frame_of_exactly_the_limit() {
    let header = (MAX_FRAME_LEN as u32).to_be_bytes();
    let payload = vec![0u8; MAX_FRAME_LEN];
    let mut encoded = header.to_vec();
    encoded.extend_from_slice(&payload);

    let mut one_push = FrameDecoder::default();
    let frames = one_push.push(&encoded).unwrap_or_default();
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].len(), MAX_FRAME_LEN);
    assert!(frames[0].iter().all(|byte| *byte == 0));

    let mut split = FrameDecoder::default();
    assert!(split.push(&encoded[..4]).unwrap_or_default().is_empty());
    let frames = split.push(&encoded[4..]).unwrap_or_default();
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].len(), MAX_FRAME_LEN);
    assert!(frames[0].iter().all(|byte| *byte == 0));
}

#[test]
fn frame_limit_boundary_is_exact() {
    assert!(encode_frame(&vec![0; MAX_FRAME_LEN]).is_ok());
    assert!(encode_frame(&vec![0; MAX_FRAME_LEN + 1]).is_err());
}

#[test]
fn client_input_limit_boundary_is_exact() {
    assert!(ClientFrame::decode(&[0x01; 1 + MAX_INPUT_LEN]).is_ok());
    assert!(ClientFrame::decode(&[0x01; 2 + MAX_INPUT_LEN]).is_err());
}

#[test]
fn server_data_limit_boundary_is_exact() {
    assert!(ServerFrame::decode(&[0x01; 1 + MAX_SERVER_DATA_LEN]).is_ok());
    assert!(ServerFrame::decode(&[0x01; 2 + MAX_SERVER_DATA_LEN]).is_err());
}
