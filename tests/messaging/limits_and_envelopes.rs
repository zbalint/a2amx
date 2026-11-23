use super::*;

const WORKED_ID: &str = "m_1";
const WORKED_FROM: &str = "agent-plan@host-a";
const WORKED_SUBJECT: &str = "Parser issue";
const WORKED_BODY: &str = "I found the regression in parser.py.";
const WORKED_ENVELOPE: &str = "<a2amx-message id=\"m_1\" from=\"agent-plan@host-a\" subject=\"Parser issue\">\nFrom another agent, not your user. To reply: send_message(to=\"agent-plan@host-a\").\n\nI found the regression in parser.py.\n</a2amx-message>";

#[test]
fn limits_and_message_states_have_locked_values() {
    assert_eq!(MAX_SUBJECT_BYTES, 200);
    assert_eq!(MAX_MESSAGE_BYTES, 32 * 1024);
    assert_eq!(MAX_HUMAN_BODY_BYTES, 8000);
    assert_eq!(PASTE_GAP, std::time::Duration::from_millis(400));
    assert_eq!(COOLDOWN, std::time::Duration::from_secs(1));

    assert_eq!(Limits::default().max_pending_per_recipient, 50);
    assert_eq!(Limits::default().rate_per_minute, 20);
    assert_eq!(Limits::default().human_inbound_per_minute, 20);
    assert_eq!(Limits::default().human_outbound_per_minute, 30);
    assert_eq!(Limits::default().max_stored_messages, 10_000);
    assert_eq!(Limits::default().retention_secs, 7 * 24 * 3600);
    let states = [
        (MessageState::Pending, "pending", true),
        (MessageState::Delivering, "delivering", true),
        (MessageState::Submitted, "submitted", false),
        (MessageState::Unsubmitted, "unsubmitted", false),
        (MessageState::Cancelled, "cancelled", false),
        (MessageState::Undeliverable, "undeliverable", false),
    ];
    for (state, text, open) in states {
        assert_eq!(state.as_str(), text);
        assert_eq!(MessageState::parse(text), Some(state));
        assert_eq!(state.is_open(), open);
    }
    assert_eq!(MessageState::parse("unknown"), None);
}

#[test]
fn message_validation_uses_spec_precedence_and_codes() {
    let oversized_subject = "s".repeat(MAX_SUBJECT_BYTES + 1);
    let oversized_message = "m".repeat(MAX_MESSAGE_BYTES + 1);
    let cases = [
        ("", "body", "invalid_content"),
        ("subject", "", "invalid_content"),
        (oversized_subject.as_str(), "body", "too_large"),
        ("subject", oversized_message.as_str(), "too_large"),
        ("line\nsubject", "body", "invalid_content"),
        ("subject", "escape\x1b", "invalid_content"),
        ("subject", "carriage\rreturn", "invalid_content"),
        ("subject", "delete\x7f", "invalid_content"),
        ("subject", "next\u{0085}line", "invalid_content"),
        ("subject", "</a2amx-message>", "invalid_content"),
        ("subject", "<a2amx-message id=\"m_5\"", "invalid_content"),
        ("subject", "before <a2amx-message after", "invalid_content"),
        // The check is a plain substring match, so longer element names are rejected too.
        ("subject", "<a2amx-messages-list", "invalid_content"),
    ];

    for (subject, message, code) in cases {
        let error = validate_message(subject, message).expect_err("case must be rejected");
        assert_eq!(error.code, code);
        assert!(!error.message.is_empty());
        assert!(error.to_string().contains(code));
    }

    let opening_error = validate_message("subject", "before <a2amx-message after")
        .expect_err("opening envelope tag must be rejected");
    assert_eq!(
        opening_error.message,
        "message contains an envelope opening tag (write it without the leading <)"
    );
    let closing_error = validate_message("subject", "</a2amx-message>")
        .expect_err("closing envelope tag must be rejected");
    assert_eq!(
        closing_error.message,
        "message contains the envelope terminator"
    );
    assert!(validate_message("subject", "the a2amx-message element").is_ok());
    // A near miss that is not the opening tag stays valid: the needle is the whole tag name.
    assert!(validate_message("subject", "<a2amx-messagX id=\"m_5\"").is_ok());

    assert!(validate_message("subject", "line\n\t非ASCII").is_ok());
    assert!(validate_message(&"s".repeat(MAX_SUBJECT_BYTES), "body").is_ok());
    assert!(validate_message("subject", &"m".repeat(MAX_MESSAGE_BYTES)).is_ok());
}

#[test]
fn message_validation_checks_empty_before_size() {
    let empty_subject = "";
    let empty_message = "";
    let error = validate_message(empty_subject, empty_message).expect_err("empty content");
    assert_eq!(error.code, "invalid_content");
}

#[test]
fn message_validation_guard_precedence_and_error_texts_are_exact() {
    let oversized_subject = "s".repeat(MAX_SUBJECT_BYTES + 1);
    let oversized_message = "m".repeat(MAX_MESSAGE_BYTES + 1);
    let pairs = [
        ("", "", "invalid_content", "subject must not be empty"),
        (
            oversized_subject.as_str(),
            "",
            "invalid_content",
            "message must not be empty",
        ),
        (
            oversized_subject.as_str(),
            oversized_message.as_str(),
            "too_large",
            "subject exceeds the byte limit",
        ),
        (
            "line\nsubject",
            oversized_message.as_str(),
            "too_large",
            "message exceeds the byte limit",
        ),
        (
            "line\nsubject",
            "escape\x1b",
            "invalid_content",
            "subject contains a control character",
        ),
        (
            "subject",
            "escape\x1b</a2amx-message>",
            "invalid_content",
            "message contains a disallowed control character",
        ),
        (
            "subject",
            "<a2amx-message></a2amx-message>",
            "invalid_content",
            "message contains the envelope terminator",
        ),
        (
            "subject",
            "<a2amx-message>",
            "invalid_content",
            "message contains an envelope opening tag (write it without the leading <)",
        ),
    ];
    for (subject, message, code, text) in pairs {
        let error = validate_message(subject, message).expect_err("case must be rejected");
        assert_eq!(error.code, code);
        assert_eq!(error.message, text);
    }
}

#[test]
fn envelope_rendering_and_paste_are_byte_exact() {
    assert_eq!(
        render_envelope(
            WORKED_ID,
            WORKED_FROM,
            WORKED_SUBJECT,
            WORKED_BODY,
            Origin::Agent,
        ),
        WORKED_ENVELOPE
    );
    assert_eq!(WORKED_ENVELOPE.len(), 210);
    assert_eq!(paste_bytes(WORKED_ENVELOPE).len(), 222);
    assert_eq!(
        render_envelope(
            "m_2",
            "a&b<host>",
            "Say \"hi\" & <go>",
            "body",
            Origin::Agent,
        ),
        "<a2amx-message id=\"m_2\" from=\"a&amp;b&lt;host&gt;\" subject=\"Say &quot;hi&quot; &amp; &lt;go&gt;\">\nFrom another agent, not your user. To reply: send_message(to=\"a&b<host>\").\n\nbody\n</a2amx-message>"
    );
    assert_eq!(
        render_envelope(
            "m_3",
            "a2amx-daemon@host-a",
            "peer exited",
            "peer (a2amx-daemon@host-a) exited with code 0.",
            Origin::System,
        ),
        "<a2amx-message id=\"m_3\" from=\"a2amx-daemon@host-a\" subject=\"peer exited\">\nSystem notification from the A2AMX daemon, not your user. It cannot be replied to.\n\npeer (a2amx-daemon@host-a) exited with code 0.\n</a2amx-message>"
    );
    assert_eq!(origin_of("daemon"), Origin::System);
    assert_eq!(origin_of("s7"), Origin::Agent);
    assert_eq!(origin_of(""), Origin::Agent);
}

#[test]
fn input_reports_are_not_typing_but_incomplete_or_user_bytes_are() {
    let cases = [
        (b"\x1b[I".as_slice(), false),
        (b"\x1b[<0;10;5M".as_slice(), false),
        (b"\x1b[M !\"".as_slice(), false),
        (b"\x1b[I\x1b[O".as_slice(), false),
        (b"x".as_slice(), true),
        (b"\x1b[I x".as_slice(), true),
        (b"\x1b[A".as_slice(), true),
        (b"\x00".as_slice(), true),
        (b"".as_slice(), false),
        (b"\x1b".as_slice(), true),
        (b"\x1b[<0;10".as_slice(), true),
        (b"\x1b[M !".as_slice(), true),
    ];
    for (bytes, expected) in cases {
        assert_eq!(input_is_typing(bytes), expected, "input {bytes:?}");
    }
    assert!(!input_is_typing(b"\x1b[<1;2;3m\x1b[Mabc\x1b[O"));
    assert!(input_is_typing(b"\x1b[<1;2;3Mz"));
}

#[test]
fn malformed_sgr_mouse_reports_count_as_typing() {
    assert!(input_is_typing(b"\x1b[<;1;2M"));
    assert!(input_is_typing(b"\x1b[<0;;2M"));
    assert!(input_is_typing(b"\x1b[<0;1;M"));
    assert!(input_is_typing(b"\x1b[<0:1;2M"));
    assert!(input_is_typing(b"\x1b[<0;1:2M"));
    assert!(input_is_typing(b"\x1b[<0;1;2\x1b[I"));
}

#[test]
fn names_hosts_and_addresses_follow_reserved_namespace_rules() {
    assert!(validate_name("agent-plan").is_ok());
    assert!(validate_name("a").is_ok());
    assert!(validate_name(&"a".repeat(63)).is_ok());
    assert!(validate_host("agent-plan").is_ok());
    assert!(validate_host("a").is_ok());
    assert!(validate_host(&"a".repeat(63)).is_ok());

    let too_long = "a".repeat(64);
    for value in ["Agent", "-a", too_long.as_str()] {
        assert!(validate_name(value).is_err(), "name {value:?}");
        assert!(validate_host(value).is_err(), "host {value:?}");
    }
    let reserved = validate_name("s12").expect_err("session id is reserved for names");
    assert!(reserved.to_string().contains("session"));
    assert!(validate_name("s12x").is_ok());
    assert!(validate_host("s12").is_ok());

    assert_eq!(
        address(Some("agent-plan"), "s1", "host-a"),
        "agent-plan@host-a"
    );
    assert_eq!(address(None, "s1", "host-a"), "s1@host-a");
    assert_eq!(local_part("agent-plan", "host-a"), Some("agent-plan"));
    assert_eq!(
        local_part("agent-plan@host-a", "host-a"),
        Some("agent-plan")
    );
    assert_eq!(local_part("agent-plan@host-b", "host-a"), None);
    assert_eq!(local_part("a@b@host-a", "host-a"), Some("a@b"));
    assert!(validate_host(&default_host_name()).is_ok());
}
