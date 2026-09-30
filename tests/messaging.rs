use a2amx::emulator::{Emulator, Size};
use std::path::Path;

use a2amx::harness::{self, Deliver, Harness};
use a2amx::messaging::{
    COOLDOWN, Limits, MAX_MESSAGE_BYTES, MAX_SUBJECT_BYTES, MessageState, PASTE_GAP, address,
    default_host_name, input_is_typing, local_part, paste_bytes, render_envelope, validate_host,
    validate_message, validate_name,
};

const WORKED_ID: &str = "m_1";
const WORKED_FROM: &str = "agent-plan@host-a";
const WORKED_SUBJECT: &str = "Parser issue";
const WORKED_BODY: &str = "I found the regression in parser.py.";
const WORKED_ENVELOPE: &str = "<a2amx-message id=\"m_1\" from=\"agent-plan@host-a\" subject=\"Parser issue\">\nFrom another agent, not your user. To reply: send_message(to=\"agent-plan@host-a\").\n\nI found the regression in parser.py.\n</a2amx-message>";

#[test]
fn limits_and_message_states_have_locked_values() {
    assert_eq!(MAX_SUBJECT_BYTES, 200);
    assert_eq!(MAX_MESSAGE_BYTES, 32 * 1024);
    assert_eq!(PASTE_GAP, std::time::Duration::from_millis(400));
    assert_eq!(COOLDOWN, std::time::Duration::from_secs(1));

    assert_eq!(Limits::default().max_pending_per_recipient, 50);
    assert_eq!(Limits::default().rate_per_minute, 20);
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
    ];

    for (subject, message, code) in cases {
        let error = validate_message(subject, message).expect_err("case must be rejected");
        assert_eq!(error.code, code);
        assert!(!error.message.is_empty());
        assert!(error.to_string().contains(code));
    }

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
fn envelope_rendering_and_paste_are_byte_exact() {
    assert_eq!(
        render_envelope(WORKED_ID, WORKED_FROM, WORKED_SUBJECT, WORKED_BODY),
        WORKED_ENVELOPE
    );
    assert_eq!(WORKED_ENVELOPE.len(), 210);
    assert_eq!(paste_bytes(WORKED_ENVELOPE).len(), 222);
    assert_eq!(
        render_envelope("m_2", "a&b<host>", "Say \"hi\" & <go>", "body"),
        "<a2amx-message id=\"m_2\" from=\"a&amp;b&lt;host&gt;\" subject=\"Say &quot;hi&quot; &amp; &lt;go&gt;\">\nFrom another agent, not your user. To reply: send_message(to=\"a&b<host>\").\n\nbody\n</a2amx-message>"
    );
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

#[test]
fn harness_defaults_and_claude_argv_wiring_are_exact() {
    assert_eq!(Harness::Claude.default_deliver(), Deliver::Auto);
    assert_eq!(Harness::Generic.default_deliver(), Deliver::Hold);

    let exe = Path::new("/tmp/a2amx binary");
    let argv = harness::wire_claude_argv(
        vec!["claude".into(), "--model".into(), "sonnet".into()],
        exe,
        true,
    );
    assert_eq!(argv[0], "claude");
    assert_eq!(argv[1], "--model");
    assert_eq!(argv[2], "sonnet");
    assert_eq!(argv[3], "--mcp-config");
    let config: serde_json::Value = serde_json::from_str(&argv[4]).expect("mcp config JSON");
    assert_eq!(
        config,
        serde_json::json!({
            "mcpServers": {
                "a2amx": {"command": "/tmp/a2amx binary", "args": ["mcp"]}
            }
        })
    );
    assert_eq!(argv[5], "--allowedTools");
    assert_eq!(
        argv[6],
        "mcp__a2amx__list_agents,mcp__a2amx__send_message,mcp__a2amx__message_status"
    );
    assert_eq!(argv[7], "--append-system-prompt");
    assert_eq!(argv[8], harness::PEER_AUTHORIZATION_PROMPT);

    let before_separator = harness::wire_claude_argv(
        vec!["claude".into(), "--".into(), "positional".into()],
        exe,
        false,
    );
    assert_eq!(before_separator[0], "claude");
    assert_eq!(before_separator[1], "--mcp-config");
    assert_eq!(before_separator[3], "--allowedTools");
    assert_eq!(before_separator[5], "--");
    assert!(
        !before_separator
            .iter()
            .any(|arg| arg == "--append-system-prompt")
    );
}

fn screen_from(bytes: &[u8]) -> a2amx::emulator::Screen {
    let mut emulator = Emulator::new(Size { cols: 40, rows: 10 });
    emulator.feed(bytes);
    emulator.screen()
}

fn f1_bytes() -> Vec<u8> {
    let rules = "─".repeat(40);
    format!("\x1b[2J\x1b[?2004h\x1b[2;1H{rules}\x1b[3;1H❯ \x1b[4;1H{rules}\x1b[3;3H\x1b[?25h")
        .into_bytes()
}

#[test]
fn claude_readiness_accepts_idle_and_mid_turn_composers() {
    let f1 = f1_bytes();
    assert!(harness::ready(Harness::Claude, &screen_from(&f1), false));

    let f2 = "\x1b[2J\x1b[?2004h\x1b[1;1H✢ Spinning… (4s)\x1b[2;1H".to_owned()
        + &"─".repeat(40)
        + "\x1b[3;1H❯ Press up to edit queued messages\x1b[4;1H"
        + &"─".repeat(40)
        + "\x1b[3;3H\x1b[?25h";
    assert!(harness::ready(
        Harness::Claude,
        &screen_from(f2.as_bytes()),
        false
    ));
}

#[test]
fn claude_readiness_rejects_each_non_ready_fixture() {
    let f1 = f1_bytes();

    let mut f4 = f1.clone();
    f4.extend_from_slice(b"\x1b[?25l");
    assert!(!harness::ready(Harness::Claude, &screen_from(&f4), false));

    let f5 = "\x1b[2J\x1b[?2004h\x1b[2;1H".to_owned()
        + &"─".repeat(40)
        + "\x1b[3;1H❯ \x1b[3;3H\x1b[?25h";
    assert!(!harness::ready(
        Harness::Claude,
        &screen_from(f5.as_bytes()),
        false
    ));

    let f6 = "\x1b[2J\x1b[?2004h\x1b[2;1H".to_owned()
        + &"─".repeat(40)
        + "\x1b[3;1H ❯ \x1b[4;1H"
        + &"─".repeat(40)
        + "\x1b[3;3H\x1b[?25h";
    assert!(!harness::ready(
        Harness::Claude,
        &screen_from(f6.as_bytes()),
        false
    ));

    let mut f7 = f1.clone();
    f7.extend_from_slice(b"\x1b[?2004l");
    assert!(!harness::ready(Harness::Claude, &screen_from(&f7), false));

    let f8 = String::from_utf8([f1.as_slice(), b"\x1b[8;1HEnter to confirm"].concat())
        .expect("ASCII suffix");
    let mut f8 = f8.into_bytes();
    f8.extend_from_slice(b"\x1b[3;3H\x1b[?25h");
    assert!(!harness::ready(Harness::Claude, &screen_from(&f8), false));

    assert!(!harness::ready(Harness::Claude, &screen_from(&f1), true));

    let f3 = "\x1b[2J\x1b[?2004h\x1b[1;1H".to_owned()
        + &"─".repeat(40)
        + "\x1b[2;2HAccessing workspace:\x1b[6;2H❯ No, exit\x1b[7;4HYes, I trust this folder\x1b[9;2HEnter to confirm · Esc to cancel\x1b[?25l";
    assert!(!harness::ready(
        Harness::Claude,
        &screen_from(f3.as_bytes()),
        false
    ));
}

#[test]
fn generic_readiness_only_uses_bracketed_paste_and_scroll_state() {
    let f1 = f1_bytes();
    assert!(harness::ready(Harness::Generic, &screen_from(&f1), false));

    let mut paste_off = f1.clone();
    paste_off.extend_from_slice(b"\x1b[?2004l");
    assert!(!harness::ready(
        Harness::Generic,
        &screen_from(&paste_off),
        false
    ));
    assert!(!harness::ready(Harness::Generic, &screen_from(&f1), true));
}
