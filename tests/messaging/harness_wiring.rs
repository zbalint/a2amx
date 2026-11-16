use super::*;

#[test]
fn harness_defaults_and_claude_argv_wiring_are_exact() {
    assert_eq!(Harness::Claude.default_deliver(), Deliver::Auto);
    assert_eq!(Harness::Generic.default_deliver(), Deliver::Hold);

    let cwd_dir = TempDir::new().expect("temporary Claude cwd");
    let config_dir = TempDir::new().expect("temporary Claude config dir");
    let cwd = cwd_dir.path();
    let exe = Path::new("/tmp/a2amx binary");
    let argv = harness::wire_claude_argv(
        vec!["claude".into(), "--model".into(), "sonnet".into()],
        exe,
        cwd,
        config_dir.path(),
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
            },
            "statusLine": {
                "type": "command",
                "command": "'/tmp/a2amx binary' statusline"
            }
        })
    );
    let peer_only = harness::wire_claude_argv(
        vec!["claude".into()],
        exe,
        cwd,
        config_dir.path(),
        true,
        None,
    )
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

    let role_only = harness::wire_claude_argv(
        vec!["claude".into()],
        exe,
        cwd,
        config_dir.path(),
        false,
        Some("consultant"),
    )
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

    let no_prompt = harness::wire_claude_argv(
        vec!["claude".into()],
        exe,
        cwd,
        config_dir.path(),
        false,
        None,
    )
    .expect("Claude argv wiring");
    assert!(
        !no_prompt
            .iter()
            .any(|argument| argument == "--append-system-prompt")
    );
    let channel_combined = harness::wire_claude_channel_argv(
        vec!["claude".into()],
        exe,
        cwd,
        config_dir.path(),
        true,
        Some("architect"),
    )
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
        config_dir.path(),
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
        config_dir.path(),
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
        Path::new("/nonexistent/a2amx-config"),
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
    let cwd_dir = TempDir::new().expect("temporary Claude cwd");
    let config_dir = TempDir::new().expect("temporary Claude config dir");
    let argv = harness::wire_claude_argv(
        vec![
            "claude".into(),
            "--settings".into(),
            r#"{"model":"opus"}"#.into(),
            "--".into(),
            "prompt".into(),
        ],
        Path::new("/usr/bin/a2amx"),
        cwd_dir.path(),
        config_dir.path(),
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
            }]},
            "statusLine": {
                "type": "command",
                "command": "'/usr/bin/a2amx' statusline"
            }
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
        Path::new("/nonexistent/a2amx-config"),
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
        Path::new("/nonexistent/a2amx-config"),
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
        Path::new("/nonexistent/a2amx-config"),
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
            Path::new("/nonexistent/a2amx-config"),
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
        Path::new("/nonexistent/a2amx-config"),
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
            Path::new("/nonexistent/a2amx-config"),
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
        Path::new("/nonexistent/a2amx-config"),
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

#[test]
fn claude_status_line_precedence_and_command_metadata_are_preserved() {
    let cwd = TempDir::new().expect("temporary Claude cwd");
    let config = TempDir::new().expect("temporary Claude config");
    std::fs::create_dir(cwd.path().join(".claude")).expect("Claude directory");
    std::fs::write(
        cwd.path().join(".claude/settings.local.json"),
        r#"{"statusLine":{"type":"command","command":"local","padding":2,"refreshInterval":4}}"#,
    )
    .expect("local settings");
    std::fs::write(
        cwd.path().join(".claude/settings.json"),
        r#"{"statusLine":{"type":"command","command":"project"}}"#,
    )
    .expect("project settings");
    std::fs::write(
        config.path().join("settings.json"),
        r#"{"statusLine":{"type":"command","command":"global"}}"#,
    )
    .expect("global settings");

    let argv = harness::wire_claude_argv(
        vec![
            "claude".into(),
            "--settings".into(),
            r#"{"statusLine":{"type":"command","command":"inline","padding":7,"refreshInterval":9}}"#.into(),
        ],
        Path::new("/usr/bin/a2amx"),
        cwd.path(),
        config.path(),
        false,
        None,
    )
    .expect("Claude argv wiring");
    let index = argv
        .iter()
        .position(|argument| argument == "--settings")
        .unwrap();
    let settings: serde_json::Value = serde_json::from_str(&argv[index + 1]).unwrap();
    assert_eq!(settings["statusLine"]["padding"], 7);
    assert_eq!(settings["statusLine"]["refreshInterval"], 9);
    assert!(
        settings["statusLine"]["command"]
            .as_str()
            .unwrap()
            .contains("inline")
    );

    let argv = harness::wire_claude_argv(
        vec!["claude".into()],
        Path::new("/usr/bin/a2amx"),
        cwd.path(),
        config.path(),
        false,
        None,
    )
    .expect("Claude argv wiring");
    let index = argv
        .iter()
        .position(|argument| argument == "--settings")
        .unwrap();
    let settings: serde_json::Value = serde_json::from_str(&argv[index + 1]).unwrap();
    assert_eq!(settings["statusLine"]["padding"], 2);
    assert_eq!(settings["statusLine"]["refreshInterval"], 4);
    assert!(
        settings["statusLine"]["command"]
            .as_str()
            .unwrap()
            .contains("local")
    );
    std::fs::remove_file(cwd.path().join(".claude/settings.local.json")).unwrap();
    let argv = harness::wire_claude_argv(
        vec!["claude".into()],
        Path::new("/usr/bin/a2amx"),
        cwd.path(),
        config.path(),
        false,
        None,
    )
    .unwrap();
    let index = argv
        .iter()
        .position(|argument| argument == "--settings")
        .unwrap();
    let settings: serde_json::Value = serde_json::from_str(&argv[index + 1]).unwrap();
    assert!(
        settings["statusLine"]["command"]
            .as_str()
            .unwrap()
            .contains("project")
    );

    std::fs::remove_file(cwd.path().join(".claude/settings.json")).unwrap();
    let argv = harness::wire_claude_argv(
        vec!["claude".into()],
        Path::new("/usr/bin/a2amx"),
        cwd.path(),
        config.path(),
        false,
        None,
    )
    .unwrap();
    let index = argv
        .iter()
        .position(|argument| argument == "--settings")
        .unwrap();
    let settings: serde_json::Value = serde_json::from_str(&argv[index + 1]).unwrap();
    assert!(
        settings["statusLine"]["command"]
            .as_str()
            .unwrap()
            .contains("global")
    );
}

#[test]
fn claude_status_line_invalid_files_and_shapes_are_fail_open() {
    let cwd = TempDir::new().expect("temporary Claude cwd");
    let config = TempDir::new().expect("temporary Claude config");
    std::fs::create_dir(cwd.path().join(".claude")).expect("Claude directory");
    std::fs::write(cwd.path().join(".claude/settings.local.json"), "{").expect("invalid local");
    std::fs::write(
        cwd.path().join(".claude/settings.json"),
        r#"{"statusLine":{"type":"other","command":"untouched"}}"#,
    )
    .expect("project settings");
    std::fs::write(config.path().join("settings.json"), "[]").expect("invalid global shape");

    let argv = harness::wire_claude_argv(
        vec!["claude".into()],
        Path::new("/usr/bin/a2amx"),
        cwd.path(),
        config.path(),
        false,
        None,
    )
    .expect("Claude argv wiring");
    let index = argv
        .iter()
        .position(|argument| argument == "--settings")
        .unwrap();
    let settings: serde_json::Value = serde_json::from_str(&argv[index + 1]).unwrap();
    assert_eq!(
        settings["statusLine"],
        serde_json::json!({"type":"other","command":"untouched"})
    );

    let argv = harness::wire_claude_argv(
        vec![
            "claude".into(),
            "--settings".into(),
            r#"{"statusLine":{"type":"command","command":"bad\u0000command"}}"#.into(),
        ],
        Path::new("/usr/bin/a2amx"),
        cwd.path(),
        config.path(),
        false,
        None,
    )
    .expect("Claude argv wiring");
    let index = argv
        .iter()
        .position(|argument| argument == "--settings")
        .unwrap();
    let settings: serde_json::Value = serde_json::from_str(&argv[index + 1]).unwrap();
    assert_eq!(
        settings["statusLine"],
        serde_json::json!({"type":"command","command":"bad\u{0}command"})
    );
}
