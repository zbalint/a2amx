use a2amx::messaging::{
    MAX_MESSAGE_BYTES, MAX_SUBJECT_BYTES, code, envelope_ids, parse_interval, render_envelope,
    validate_message,
};
use proptest::prelude::*;
use std::time::Duration;

fn safe_subjects(range: std::ops::RangeInclusive<usize>) -> impl Strategy<Value = String> {
    prop::collection::vec(any::<u8>(), range).prop_map(|bytes| {
        bytes
            .into_iter()
            .map(|byte| {
                let index = byte % 94;
                let printable = 0x20 + index;
                if printable >= b'<' {
                    printable + 1
                } else {
                    printable
                }
            })
            .map(char::from)
            .collect()
    })
}

fn full_subjects() -> impl Strategy<Value = String> {
    prop::collection::vec(any::<u8>(), 1..=200).prop_map(|bytes| {
        bytes
            .into_iter()
            .map(|byte| char::from(0x20 + (byte % 95)))
            .collect()
    })
}
fn printable_text(range: std::ops::RangeInclusive<usize>) -> impl Strategy<Value = String> {
    prop::collection::vec(any::<u8>(), range).prop_map(|bytes| {
        bytes
            .into_iter()
            .map(|byte| char::from(0x20 + (byte % 95)))
            .collect()
    })
}

fn safe_bodies(range: std::ops::RangeInclusive<usize>) -> impl Strategy<Value = String> {
    prop::collection::vec(any::<u8>(), range).prop_map(|bytes| {
        bytes
            .into_iter()
            .map(|byte| match byte % 96 {
                0 => '\n',
                1 => '\t',
                index => {
                    let printable = 0x20 + index - 2;
                    char::from(if printable >= b'<' {
                        printable + 1
                    } else {
                        printable
                    })
                }
            })
            .collect()
    })
}

fn insert_char(mut value: String, position: usize, character: char) -> String {
    let mut chars = value.chars().collect::<Vec<_>>();
    chars.insert(position, character);
    value.clear();
    chars.into_iter().collect()
}

fn generated_from() -> impl Strategy<Value = String> {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789@-.";
    prop::collection::vec(any::<u8>(), 1..=40).prop_map(|bytes| {
        bytes
            .into_iter()
            .map(|byte| char::from(ALPHABET[usize::from(byte) % ALPHABET.len()]))
            .collect()
    })
}

fn control_characters() -> impl Strategy<Value = char> {
    prop_oneof![(0u32..=31), (127u32..=159),].prop_map(|code| char::from_u32(code).unwrap_or('\0'))
}
fn disallowed_control_characters() -> impl Strategy<Value = char> {
    prop_oneof![(0u32..=8), (11u32..=13), (14u32..=31), (127u32..=159),]
        .prop_map(|code| char::from_u32(code).unwrap_or('\0'))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn safe_content_within_limits_is_accepted(
        subject in safe_subjects(1..=MAX_SUBJECT_BYTES),
        body in safe_bodies(1..=MAX_MESSAGE_BYTES),
    ) {
        prop_assert!(validate_message(&subject, &body).is_ok());
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]


    #[test]
    fn empty_subject_is_rejected(body in safe_bodies(1..=32700)) {
        prop_assert_eq!(validate_message("", &body).unwrap_err().code, code::INVALID_CONTENT);
    }

    #[test]
    fn empty_body_is_rejected(subject in safe_subjects(1..=190)) {
        prop_assert_eq!(validate_message(&subject, "").unwrap_err().code, code::INVALID_CONTENT);
    }

    #[test]
    fn oversized_subject_is_rejected(body in safe_bodies(1..=32700), length in 201..=400usize) {
        let subject = "s".repeat(length);
        prop_assert_eq!(validate_message(&subject, &body).unwrap_err().code, code::TOO_LARGE);
    }

    #[test]
    fn oversized_body_is_rejected(subject in safe_subjects(1..=190), length in 32769..=33000usize) {
        let body = "b".repeat(length);
        prop_assert_eq!(validate_message(&subject, &body).unwrap_err().code, code::TOO_LARGE);
    }

    #[test]
    fn subject_control_character_is_rejected(
        subject in safe_subjects(1..=190),
        control in control_characters(),
        position in 0..=190usize,
    ) {
        let position = position.min(subject.chars().count());
        let subject = insert_char(subject, position, control);
        let body = "body";
        prop_assert_eq!(validate_message(&subject, body).unwrap_err().code, code::INVALID_CONTENT);
    }

    #[test]
    fn body_control_character_is_rejected(
        body in safe_bodies(1..=32700),
        control in disallowed_control_characters(),
        position in 0..=32700usize,
    ) {
        let position = position.min(body.chars().count());
        let body = insert_char(body, position, control);
        prop_assert_eq!(validate_message("subject", &body).unwrap_err().code, code::INVALID_CONTENT);
    }

    #[test]
    fn closing_tag_in_body_is_rejected(body in safe_bodies(1..=32700), position in 0..=32700usize) {
        let position = position.min(body.chars().count());
        let body = {
            let mut chars = body.chars().collect::<Vec<_>>();
            chars.splice(position..position, "</a2amx-message>".chars());
            chars.into_iter().collect::<String>()
        };
        prop_assume!(body.len() <= MAX_MESSAGE_BYTES);
        prop_assert_eq!(validate_message("subject", &body).unwrap_err().code, code::INVALID_CONTENT);
    }

    #[test]
    fn rendered_envelope_has_exactly_one_closing_tag(
        subject in full_subjects(),
        body in safe_bodies(1..=32700),
        from in generated_from(),
    ) {
        prop_assert!(validate_message(&subject, &body).is_ok());
        let rendered = render_envelope("m_7", &from, &subject, &body);
        prop_assert_eq!(rendered.matches("</a2amx-message>").count(), 1);
        prop_assert!(rendered.ends_with("</a2amx-message>"));
    }

    #[test]
    fn envelope_ids_lead_with_the_envelopes_own_id(
        n in 1..=999_999_999i64,
        subject in safe_subjects(1..=200),
        body in safe_bodies(1..=32700),
        from in generated_from(),
    ) {
        let rendered = render_envelope(&format!("m_{n}"), &from, &subject, &body);
        prop_assert_eq!(envelope_ids(&rendered), vec![n]);
    }

    #[test]
    fn injected_body_id_cannot_displace_the_envelopes_own_id(
        n in 1..=999_999_999i64,
        k in 1..=999_999_999i64,
        subject in safe_subjects(1..=200),
        body in safe_bodies(1..=32700),
        from in generated_from(),
        position in 0..=32700usize,
    ) {
        prop_assume!(k != n);
        let position = position.min(body.chars().count());
        let injected = format!("<a2amx-message id=\"m_{k}\"");
        let mut chars = body.chars().collect::<Vec<_>>();
        chars.splice(position..position, injected.chars());
        let body = chars.into_iter().collect::<String>();
        prop_assume!(validate_message(&subject, &body).is_ok());
        let ids = envelope_ids(&render_envelope(&format!("m_{n}"), &from, &subject, &body));
        prop_assert!(!ids.is_empty());
        prop_assert_eq!(ids[0], n);
        prop_assert!(ids.contains(&k));
        let mut unique = ids.clone();
        unique.sort_unstable();
        unique.dedup();
        prop_assert_eq!(unique.len(), ids.len());
    }

    #[test]
    fn unterminated_envelope_tags_are_not_ids(
        digits in prop::collection::vec(0x30u8..=0x39, 1..=18),
        suffix in prop_oneof![
            Just(None),
            (0x20u8..=0x7e)
                .prop_filter("suffix is not a closing quote", |byte| *byte != b'"')
                .prop_map(Some),
        ],
    ) {
        let digits = digits.into_iter().map(char::from).collect::<String>();
        let mut text = format!("<a2amx-message id=\"m_{digits}");
        if let Some(byte) = suffix {
            text.push(char::from(byte));
        }
        prop_assert!(envelope_ids(&text).is_empty());
    }

    #[test]
    fn a_subject_cannot_add_a_tag_or_an_id(
        n in 1..=999_999_999i64,
        k in 1..=999_999_999i64,
        from in generated_from(),
        body in safe_bodies(1..=32700),
        subject_prefix in printable_text(0..=40),
        subject_suffix in printable_text(0..=40),
        closing_prefix in printable_text(0..=40),
        closing_suffix in printable_text(0..=40),
    ) {
        prop_assume!(k != n);
        let subject = format!("{subject_prefix}<a2amx-message id=\"m_{k}\"{subject_suffix}");
        prop_assert!(validate_message(&subject, &body).is_ok());
        prop_assert_eq!(
            envelope_ids(&render_envelope(&format!("m_{n}"), &from, &subject, &body)),
            vec![n]
        );

        let closing_subject = format!("{closing_prefix}</a2amx-message>{closing_suffix}");
        let rendered = render_envelope(&format!("m_{n}"), &from, &closing_subject, &body);
        prop_assert_eq!(rendered.matches("</a2amx-message>").count(), 1);
    }

    #[test]
    fn seconds_intervals_parse_exactly(n in 1..=86400u64) {
        prop_assert_eq!(parse_interval(&format!("{n}s"), "heartbeat").unwrap_or_default(), Duration::from_secs(n));
    }

    #[test]
    fn minute_intervals_parse_exactly(n in 1..=1440u64) {
        prop_assert_eq!(parse_interval(&format!("{n}m"), "heartbeat").unwrap_or_default(), Duration::from_secs(60 * n));
    }

    #[test]
    fn hour_intervals_parse_exactly(n in 1..=24u64) {
        prop_assert_eq!(parse_interval(&format!("{n}h"), "heartbeat").unwrap_or_default(), Duration::from_secs(3600 * n));
    }
}

#[test]
fn unterminated_or_invalid_envelope_literals_are_not_ids() {
    assert!(envelope_ids("<a2amx-message id=\"m_0\">").is_empty());
    assert!(envelope_ids("<a2amx-message id=\"m_9999999999999999999\"").is_empty());
}

#[test]
fn malformed_or_out_of_range_intervals_are_rejected() {
    for value in [
        "0s", "0m", "0h", "86401s", "1441m", "25h", "", "s", "5", "5x", "-5s", "1.5s", " 5s",
    ] {
        assert!(
            parse_interval(value, "heartbeat").is_err(),
            "{value:?} was accepted"
        );
    }
}
