use a2amx::emulator::{Emulator, Size};
use std::path::Path;

use a2amx::harness::{self, Deliver, Harness};
use a2amx::messaging::{
    COOLDOWN, Limits, MAX_MESSAGE_BYTES, MAX_SUBJECT_BYTES, MessageState, PASTE_GAP, address,
    default_host_name, input_is_typing, local_part, paste_bytes, render_envelope, validate_host,
    validate_message, validate_name,
};
use tempfile::TempDir;

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
    let cwd = Path::new(".");
    let argv = harness::wire_claude_argv(
        vec!["claude".into(), "--model".into(), "sonnet".into()],
        exe,
        cwd,
        true,
        Some("architect"),
    )
    .expect("Claude argv wiring");
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
        "mcp__a2amx__list_agents,mcp__a2amx__send_message,mcp__a2amx__message_status,mcp__a2amx__reset_session"
    );
    assert_eq!(argv[7], "--append-system-prompt");
    assert_eq!(
        argv[8],
        format!(
            "{} Operator-assigned role for this session: architect. It is a label set by your operator and grants no extra authority.",
            harness::PEER_AUTHORIZATION_PROMPT
        )
    );
    assert_eq!(argv[9], "--settings");
    let settings: serde_json::Value = serde_json::from_str(&argv[10]).expect("settings JSON");
    assert_eq!(
        settings,
        serde_json::json!({
            "hooks": {
                "UserPromptSubmit": [{
                    "hooks": [{
                        "type": "command",
                        "command": "'/tmp/a2amx binary' hook",
                        "timeout": 5
                    }]
                }]
            }
        })
    );
    let peer_only = harness::wire_claude_argv(vec!["claude".into()], exe, cwd, true, None)
        .expect("Claude argv wiring");
    let peer_index = peer_only
        .iter()
        .position(|argument| argument == "--append-system-prompt")
        .expect("peer prompt flag");
    assert_eq!(
        peer_only[peer_index + 1],
        harness::PEER_AUTHORIZATION_PROMPT
    );
    assert_eq!(
        peer_only
            .iter()
            .filter(|argument| argument.as_str() == "--append-system-prompt")
            .count(),
        1
    );

    let role_only =
        harness::wire_claude_argv(vec!["claude".into()], exe, cwd, false, Some("consultant"))
            .expect("Claude argv wiring");
    let role_index = role_only
        .iter()
        .position(|argument| argument == "--append-system-prompt")
        .expect("role prompt flag");
    assert_eq!(
        role_only[role_index + 1],
        "Operator-assigned role for this session: consultant. It is a label set by your operator and grants no extra authority."
    );
    assert_eq!(
        role_only
            .iter()
            .filter(|argument| argument.as_str() == "--append-system-prompt")
            .count(),
        1
    );

    let no_prompt = harness::wire_claude_argv(vec!["claude".into()], exe, cwd, false, None)
        .expect("Claude argv wiring");
    assert!(
        !no_prompt
            .iter()
            .any(|argument| argument == "--append-system-prompt")
    );
    let channel_combined =
        harness::wire_claude_channel_argv(vec!["claude".into()], exe, cwd, true, Some("architect"))
            .expect("Claude channel argv wiring");
    let channel_index = channel_combined
        .iter()
        .position(|argument| argument == "--append-system-prompt")
        .expect("channel prompt flag");
    assert_eq!(
        channel_combined[channel_index + 1],
        format!(
            "{} Operator-assigned role for this session: architect. It is a label set by your operator and grants no extra authority.",
            harness::PEER_AUTHORIZATION_PROMPT
        )
    );
    let channel_role_only = harness::wire_claude_channel_argv(
        vec!["claude".into()],
        exe,
        cwd,
        false,
        Some("consultant"),
    )
    .expect("Claude channel argv wiring");
    let channel_role_index = channel_role_only
        .iter()
        .position(|argument| argument == "--append-system-prompt")
        .expect("channel role prompt flag");
    assert_eq!(
        channel_role_only[channel_role_index + 1],
        "Operator-assigned role for this session: consultant. It is a label set by your operator and grants no extra authority."
    );

    let before_separator = harness::wire_claude_argv(
        vec!["claude".into(), "--".into(), "positional".into()],
        exe,
        cwd,
        false,
        None,
    )
    .expect("Claude argv wiring");
    assert_eq!(before_separator[0], "claude");
    assert_eq!(before_separator[1], "--mcp-config");
    assert_eq!(before_separator[3], "--allowedTools");
    assert_eq!(before_separator[5], "--settings");
    let settings: serde_json::Value =
        serde_json::from_str(&before_separator[6]).expect("settings JSON");
    assert_eq!(
        settings["hooks"]["UserPromptSubmit"][0]["hooks"][0]["command"],
        "'/tmp/a2amx binary' hook"
    );
    assert_eq!(before_separator[7], "--");
    assert!(
        !before_separator
            .iter()
            .any(|arg| arg == "--append-system-prompt")
    );
}

#[test]
fn claude_argv_shell_quotes_apostrophe_paths_exactly() {
    let argv = harness::wire_claude_argv(
        vec!["claude".into()],
        Path::new("/tmp/it's/a2amx"),
        Path::new("."),
        false,
        None,
    )
    .expect("Claude argv wiring");
    assert_eq!(argv[1], "--mcp-config");
    assert_eq!(argv[3], "--allowedTools");
    assert_eq!(argv[5], "--settings");
    let settings: serde_json::Value = serde_json::from_str(&argv[6]).expect("settings JSON");
    assert_eq!(
        settings["hooks"]["UserPromptSubmit"][0]["hooks"][0]["command"],
        "'/tmp/it'\\''s/a2amx' hook"
    );
}

#[test]
fn claude_settings_inline_preserves_user_settings_and_prompt() {
    let argv = harness::wire_claude_argv(
        vec![
            "claude".into(),
            "--settings".into(),
            r#"{"model":"opus"}"#.into(),
            "--".into(),
            "prompt".into(),
        ],
        Path::new("/usr/bin/a2amx"),
        Path::new("."),
        false,
        None,
    )
    .expect("Claude argv wiring");
    assert_eq!(
        argv.iter()
            .filter(|argument| *argument == "--settings")
            .count(),
        1
    );
    assert_eq!(&argv[1..2], ["--mcp-config"]);
    assert_eq!(argv[5], "--settings");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&argv[6]).expect("merged settings JSON"),
        serde_json::json!({
            "model": "opus",
            "hooks": {"UserPromptSubmit": [{
                "hooks": [{"type": "command", "command": "'/usr/bin/a2amx' hook", "timeout": 5}]
            }]}
        })
    );
    assert_eq!(&argv[7..], ["--", "prompt"]);
}

#[test]
fn claude_settings_equal_form_and_whitespace_are_inline() {
    let equal = harness::wire_claude_argv(
        vec![
            "claude".into(),
            r#"--settings={"model":"opus"}"#.into(),
            "--".into(),
            "prompt".into(),
        ],
        Path::new("/usr/bin/a2amx"),
        Path::new("."),
        false,
        None,
    )
    .expect("Claude argv wiring");
    let settings_index = equal
        .iter()
        .position(|argument| argument == "--settings")
        .expect("settings flag");
    let settings: serde_json::Value =
        serde_json::from_str(&equal[settings_index + 1]).expect("merged settings JSON");
    assert_eq!(settings["model"], "opus");
    assert!(
        !equal
            .iter()
            .any(|argument| argument.starts_with("--settings="))
    );

    let whitespace = harness::wire_claude_argv(
        vec!["claude".into(), "--settings".into(), r#" {"a":1} "#.into()],
        Path::new("/usr/bin/a2amx"),
        Path::new("."),
        false,
        None,
    )
    .expect("Claude argv wiring");
    let settings_index = whitespace
        .iter()
        .position(|argument| argument == "--settings")
        .expect("settings flag");
    let settings: serde_json::Value =
        serde_json::from_str(&whitespace[settings_index + 1]).expect("merged settings JSON");
    assert_eq!(settings["a"], 1);
}

#[test]
fn claude_settings_existing_hooks_are_preserved_and_extended() {
    let argv = harness::wire_claude_argv(
        vec![
            "claude".into(),
            "--settings".into(),
            r#"{"disableAllHooks":true,"hooks":{"UserPromptSubmit":[{"hooks":[{"type":"command","command":"x"}]}],"Stop":[1]}}"#.into(),
        ],
        Path::new("/usr/bin/a2amx"),
        Path::new("."),
        false,
        None,
    )
    .expect("Claude argv wiring");
    let settings_index = argv
        .iter()
        .position(|argument| argument == "--settings")
        .expect("settings flag");
    let settings: serde_json::Value =
        serde_json::from_str(&argv[settings_index + 1]).expect("merged settings JSON");
    assert_eq!(settings["disableAllHooks"], true);
    assert_eq!(settings["hooks"]["Stop"], serde_json::json!([1]));
    assert_eq!(
        settings["hooks"]["UserPromptSubmit"]
            .as_array()
            .expect("UserPromptSubmit array")
            .len(),
        2
    );
    assert_eq!(
        settings["hooks"]["UserPromptSubmit"][0],
        serde_json::json!({"hooks":[{"type":"command","command":"x"}]})
    );
    assert_eq!(
        settings["hooks"]["UserPromptSubmit"][1]["hooks"][0]["command"],
        "'/usr/bin/a2amx' hook"
    );
}

#[test]
fn claude_settings_file_forms_resolve_against_session_cwd() {
    let directory = TempDir::new().expect("temporary settings directory");
    let file = directory.path().join("settings.json");
    std::fs::write(&file, r#"{"model":"opus","env":{"A2AMX_TEST":"visible"}}"#)
        .expect("settings file");

    for value in [
        file.to_string_lossy().into_owned(),
        "settings.json".to_owned(),
    ] {
        let argv = harness::wire_claude_argv(
            vec!["claude".into(), "--settings".into(), value],
            Path::new("/usr/bin/a2amx"),
            directory.path(),
            false,
            None,
        )
        .expect("Claude argv wiring");
        let settings_index = argv
            .iter()
            .position(|argument| argument == "--settings")
            .expect("settings flag");
        let settings: serde_json::Value =
            serde_json::from_str(&argv[settings_index + 1]).expect("merged settings JSON");
        assert_eq!(settings["model"], "opus");
        assert_eq!(settings["env"]["A2AMX_TEST"], "visible");
        assert_eq!(
            settings["hooks"]["UserPromptSubmit"]
                .as_array()
                .expect("UserPromptSubmit array")
                .len(),
            1
        );
    }
}

#[test]
fn claude_settings_last_occurrence_wins() {
    let argv = harness::wire_claude_argv(
        vec![
            "claude".into(),
            "--settings".into(),
            r#"{"model":"first"}"#.into(),
            "--settings={\"model\":\"last\"}".into(),
        ],
        Path::new("/usr/bin/a2amx"),
        Path::new("."),
        false,
        None,
    )
    .expect("Claude argv wiring");
    assert_eq!(
        argv.iter()
            .filter(|argument| *argument == "--settings")
            .count(),
        1
    );
    let settings_index = argv
        .iter()
        .position(|argument| argument == "--settings")
        .expect("settings flag");
    let settings: serde_json::Value =
        serde_json::from_str(&argv[settings_index + 1]).expect("merged settings JSON");
    assert_eq!(settings["model"], "last");
}

#[test]
fn claude_settings_errors_fail_with_settings_context() {
    let directory = TempDir::new().expect("temporary settings directory");
    let invalid_file = directory.path().join("invalid.json");
    std::fs::write(&invalid_file, "{").expect("invalid settings file");
    let array_file = directory.path().join("array.json");
    std::fs::write(&array_file, "[1]").expect("array settings file");
    let error = |argv: Vec<String>| {
        let error = harness::wire_claude_argv(
            argv,
            Path::new("/usr/bin/a2amx"),
            directory.path(),
            false,
            None,
        )
        .expect_err("settings input should fail");
        assert!(
            error.to_string().contains("--settings"),
            "error lacked --settings context: {error}"
        );
    };

    error(vec![
        "claude".into(),
        "--settings".into(),
        "missing.json".into(),
    ]);
    error(vec![
        "claude".into(),
        "--settings".into(),
        "invalid.json".into(),
    ]);
    error(vec!["claude".into(), "--settings".into(), "{bad".into()]);
    error(vec!["claude".into(), "--settings".into(), "{bad}".into()]);
    error(vec![
        "claude".into(),
        "--settings".into(),
        "array.json".into(),
    ]);
    error(vec![
        "claude".into(),
        "--settings".into(),
        r#"{"hooks":3}"#.into(),
    ]);
    error(vec![
        "claude".into(),
        "--settings".into(),
        r#"{"hooks":{"UserPromptSubmit":"x"}}"#.into(),
    ]);
    error(vec!["claude".into(), "--settings".into()]);
    error(vec!["claude".into(), "--settings=".into()]);
}

#[test]
fn claude_channel_settings_merge_matches_plain_launch() {
    let argv = harness::wire_claude_channel_argv(
        vec![
            "claude".into(),
            "--settings".into(),
            r#"{"model":"opus"}"#.into(),
            "--".into(),
            "prompt".into(),
        ],
        Path::new("/usr/bin/a2amx"),
        Path::new("."),
        false,
        None,
    )
    .expect("Claude channel argv wiring");
    assert_eq!(
        argv.iter()
            .filter(|argument| *argument == "--settings")
            .count(),
        1
    );
    let settings_index = argv
        .iter()
        .position(|argument| argument == "--settings")
        .expect("settings flag");
    let settings: serde_json::Value =
        serde_json::from_str(&argv[settings_index + 1]).expect("merged settings JSON");
    assert_eq!(settings["model"], "opus");
    assert_eq!(
        settings["hooks"]["UserPromptSubmit"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    assert_eq!(&argv[argv.len() - 2..], ["--", "prompt"]);
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
fn claude_paste_view_expands_each_tab_to_four_spaces() {
    assert_eq!(
        harness::paste_view(Harness::Claude, "a\tb\n\tc"),
        "a    b\n    c"
    );
    assert_eq!(harness::paste_view(Harness::Claude, "no tabs"), "no tabs");
    assert_eq!(harness::paste_view(Harness::Generic, "a\tb"), "a\tb");
}

#[test]
fn claude_paste_view_escapes_wrapper_tags_only() {
    let cases = [
        (
            "x\n<pasted_content id=\"abc\">\ny",
            "x\n<\\pasted_content id=\"abc\">\ny",
        ),
        (
            "x\n</pasted_content id=\"abc\">\ny",
            "x\n<\\/pasted_content id=\"abc\">\ny",
        ),
        ("x\n<pasted_content>\ny", "x\n<\\pasted_content>\ny"),
        (
            "x\n<PASTED_CONTENT id=\"a\">\ny",
            "x\n<\\PASTED_CONTENT id=\"a\">\ny",
        ),
        ("x\n<pasted_content\ny", "x\n<\\pasted_content\ny"),
        (
            "x <pasted_content id=\"abc\"> y\nz",
            "x <\\pasted_content id=\"abc\"> y\nz",
        ),
        ("x\npasted_content bare\ny", "x\npasted_content bare\ny"),
        (
            "x\n<a2amx-message id=\"m_1\" from=\"a@b\">\ny",
            "x\n<a2amx-message id=\"m_1\" from=\"a@b\">\ny",
        ),
        ("x\n</a2amx-message>\ny", "x\n</a2amx-message>\ny"),
        (
            "x\n<channel source=\"s\">\ny",
            "x\n<channel source=\"s\">\ny",
        ),
        ("x\n<system-reminder>\ny", "x\n<system-reminder>\ny"),
        ("x\n<command-name>\ny", "x\n<command-name>\ny"),
        (
            "x\n<\\pasted_content already\ny",
            "x\n<\\pasted_content already\ny",
        ),
    ];
    for (input, expected) in cases {
        assert_eq!(harness::paste_view(Harness::Claude, input), expected);
        assert_eq!(harness::paste_view(Harness::Generic, input), input);
    }
    assert_eq!(
        harness::paste_view(Harness::Claude, "é\t<pasted_content> </PaStEd_CoNtEnT>界"),
        "é    <\\pasted_content> <\\/PaStEd_CoNtEnT>界"
    );
}

#[test]
fn claude_readiness_accepts_the_non_breaking_space_after_the_glyph() {
    // Claude Code 2.1.286 draws its composer as the glyph plus U+00A0, not a plain space.
    let rules = "─".repeat(40);
    let bytes = format!(
        "\x1b[2J\x1b[?2004h\x1b[2;1H{rules}\x1b[3;1H❯\u{a0}\x1b[4;1H{rules}\x1b[3;3H\x1b[?25h"
    )
    .into_bytes();
    assert!(harness::ready(Harness::Claude, &screen_from(&bytes), false));
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

#[test]
fn paste_wrappers_are_unwrapped_without_changing_malformed_text() {
    use a2amx::messaging::unwrap_pastes;
    let cases = [
        (
            "\n\n<pasted_content id=\"458d\">\nalpha\nbeta\n</pasted_content id=\"458d\">\n",
            "alpha\nbeta",
        ),
        (
            "hello \n\n<pasted_content id=\"458d\">\nalpha\n</pasted_content id=\"458d\">\n",
            "hello alpha",
        ),
        (
            "a\n\n<pasted_content id=\"1\">\nX\n</pasted_content id=\"1\">\nb\n\n<pasted_content id=\"2\">\nY\n</pasted_content id=\"2\">\n",
            "aXbY",
        ),
        (
            "\n\n<pasted_content id=\"458d\">\nalpha\n",
            "\n\n<pasted_content id=\"458d\">\nalpha\n",
        ),
        (
            "\n\n<pasted_content id=\"458d\">\nalpha\n</pasted_content id=\"zzzz\">\n",
            "\n\n<pasted_content id=\"458d\">\nalpha\n</pasted_content id=\"zzzz\">\n",
        ),
        ("plain text", "plain text"),
        (
            "é\n\n<pasted_content id=\"\">\nx\n</pasted_content id=\"\">\n",
            "é\n\n<pasted_content id=\"\">\nx\n</pasted_content id=\"\">\n",
        ),
    ];
    for (input, expected) in cases {
        assert_eq!(unwrap_pastes(input), expected);
    }
}

#[test]
fn paste_wrappers_trimmed_at_the_prompt_ends_are_still_unwrapped() {
    use a2amx::messaging::unwrap_pastes;
    // A message queued mid-turn reaches the hook with the prompt's ends trimmed.
    let cases = [
        (
            "<pasted_content id=\"7c5f\">\nalpha\n</pasted_content id=\"7c5f\">",
            "alpha",
        ),
        (
            "<pasted_content id=\"1\">\nX\n</pasted_content id=\"1\">\n",
            "X",
        ),
        (
            "\n\n<pasted_content id=\"1\">\nX\n</pasted_content id=\"1\">",
            "X",
        ),
        // Text glued to the tags is not a wrapper.
        (
            "hello <pasted_content id=\"1\">\nX\n</pasted_content id=\"1\">",
            "hello <pasted_content id=\"1\">\nX\n</pasted_content id=\"1\">",
        ),
        (
            "<pasted_content id=\"1\">\nX\n</pasted_content id=\"1\">tail",
            "<pasted_content id=\"1\">\nX\n</pasted_content id=\"1\">tail",
        ),
    ];
    for (input, expected) in cases {
        assert_eq!(unwrap_pastes(input), expected);
    }
}

#[test]
fn envelope_ids_require_tags_positive_bounded_digits_and_preserve_order() {
    use a2amx::messaging::envelope_ids;
    assert_eq!(
        envelope_ids("<a2amx-message id=\"m_12\" from=\"x\">"),
        vec![12]
    );
    assert_eq!(
        envelope_ids(
            "<a2amx-message id=\"m_3\"><a2amx-message id=\"m_3\"><a2amx-message id=\"m_7\">"
        ),
        vec![3, 7]
    );
    assert_eq!(
        envelope_ids(
            "<a2amx-message id=\"m_\"><a2amx-message id=\"m_x1\"><a2amx-message id=\"m_0\"> m_12"
        ),
        Vec::<i64>::new()
    );
    assert_eq!(
        envelope_ids(
            "<a2amx-message id=\"m_999999999999999999\"><a2amx-message id=\"m_1000000000000000000\">"
        ),
        vec![999_999_999_999_999_999]
    );
}

#[test]
fn restored_drafts_drop_controls_but_keep_newlines_tabs_and_unicode() {
    use a2amx::messaging::sanitize_draft;
    assert_eq!(sanitize_draft("a\u{1b}b\rc\0d\u{7f}e\u{85}f"), "abcdef");
    assert_eq!(sanitize_draft("x\ny\tz é"), "x\ny\tz é");
}

#[test]
fn team_visibility_is_symmetric_and_defaults_to_public_for_ungrouped() {
    use a2amx::messaging::{Party, TeamScope, visible};

    let private_a = TeamScope {
        name: "a".into(),
        private: true,
        allow: Vec::new(),
        agents: Vec::new(),
    };
    let private_b = TeamScope {
        name: "b".into(),
        private: true,
        allow: Vec::new(),
        agents: Vec::new(),
    };
    let public_a = TeamScope {
        name: "a".into(),
        private: false,
        allow: Vec::new(),
        agents: Vec::new(),
    };
    let allow_a = TeamScope {
        name: "a".into(),
        private: true,
        allow: vec!["b".into()],
        agents: Vec::new(),
    };
    let allow_b = TeamScope {
        name: "b".into(),
        private: true,
        allow: vec!["a".into()],
        agents: Vec::new(),
    };
    let agent_a = TeamScope {
        name: "a2amx".into(),
        private: true,
        allow: Vec::new(),
        agents: vec!["saltmdb-architect".into()],
    };
    let agent_b = TeamScope {
        name: "saltmdb".into(),
        private: true,
        allow: Vec::new(),
        agents: vec!["a2amx-architect".into()],
    };
    let agent_a_developer = TeamScope {
        name: "a2amx".into(),
        private: true,
        allow: Vec::new(),
        agents: Vec::new(),
    };
    let one_sided_a = TeamScope {
        name: "a2amx".into(),
        private: true,
        allow: Vec::new(),
        agents: vec!["saltmdb-architect".into()],
    };
    let one_sided_b = TeamScope {
        name: "saltmdb".into(),
        private: true,
        allow: Vec::new(),
        agents: Vec::new(),
    };
    for (a, b, expected) in [
        (
            Party {
                name: None,
                team: None,
            },
            Party {
                name: None,
                team: None,
            },
            true,
        ),
        (
            Party {
                name: Some("plain"),
                team: None,
            },
            Party {
                name: Some("a"),
                team: Some(&private_a),
            },
            true,
        ),
        (
            Party {
                name: None,
                team: None,
            },
            Party {
                name: Some("b"),
                team: Some(&private_b),
            },
            true,
        ),
        (
            Party {
                name: Some("a"),
                team: Some(&public_a),
            },
            Party {
                name: Some("b"),
                team: Some(&private_b),
            },
            true,
        ),
        (
            Party {
                name: Some("a"),
                team: Some(&private_a),
            },
            Party {
                name: Some("a2"),
                team: Some(&private_a),
            },
            true,
        ),
        (
            Party {
                name: Some("a"),
                team: Some(&private_a),
            },
            Party {
                name: Some("b"),
                team: Some(&private_b),
            },
            false,
        ),
        (
            Party {
                name: Some("a"),
                team: Some(&allow_a),
            },
            Party {
                name: Some("b"),
                team: Some(&private_b),
            },
            false,
        ),
        (
            Party {
                name: Some("a"),
                team: Some(&allow_a),
            },
            Party {
                name: Some("b"),
                team: Some(&allow_b),
            },
            true,
        ),
        (
            Party {
                name: Some("a2amx-architect"),
                team: Some(&agent_a),
            },
            Party {
                name: Some("saltmdb-architect"),
                team: Some(&agent_b),
            },
            true,
        ),
        (
            Party {
                name: Some("a2amx-developer"),
                team: Some(&agent_a_developer),
            },
            Party {
                name: Some("saltmdb-architect"),
                team: Some(&agent_b),
            },
            false,
        ),
        (
            Party {
                name: Some("a2amx-architect"),
                team: Some(&one_sided_a),
            },
            Party {
                name: Some("saltmdb-architect"),
                team: Some(&one_sided_b),
            },
            false,
        ),
        (
            Party {
                name: None,
                team: Some(&agent_a),
            },
            Party {
                name: Some("saltmdb-architect"),
                team: Some(&agent_b),
            },
            false,
        ),
    ] {
        assert_eq!(visible(a, b), expected);
        assert_eq!(visible(b, a), expected);
    }
}

#[test]
fn role_validation_accepts_architect_and_rejects_invalid_shapes() {
    use a2amx::messaging::validate_role;

    assert!(validate_role("architect").is_ok());
    for role in ["", "a\nb", " architect", "architect "] {
        assert!(validate_role(role).is_err(), "{role:?}");
    }
    assert!(validate_role(&"a".repeat(65)).is_err());
}
