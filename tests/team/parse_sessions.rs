use super::*;

#[test]
fn parse_preserves_commands_cwd_order_and_attach_selection() {
    let actual = team::parse(
        r#"
    [[session]]
    name = "architect"
    command = ["claude"]
    role = "architect"
    attach = true

    [[session]]
    name = "developer"
    command = ["omp"]
    cwd = "."
    "#,
        std::path::Path::new("/work/demo"),
        false,
    )
    .unwrap();
    assert_eq!(
        actual,
        vec![
            TeamSession {
                name: "demo-architect".into(),
                command: vec!["claude".into()],
                cwd: None,
                attach: true,
                reset: None,
                control_from: Vec::new(),
                watch: Vec::new(),
                heartbeat: None,
                activity: None,
                role: Some("architect".into()),
                remote: None,
                allow_agents: Vec::new(),
                confine: None,
                auto: false,
                team: Some(a2amx::messaging::TeamScope {
                    name: "demo".into(),
                    private: true,
                    allow: Vec::new(),
                    agents: Vec::new(),
                }),
            },
            TeamSession {
                name: "demo-developer".into(),
                command: vec!["omp".into()],
                cwd: Some(".".into()),
                attach: false,
                reset: None,
                control_from: Vec::new(),
                watch: Vec::new(),
                heartbeat: None,
                activity: None,
                role: None,
                remote: None,
                allow_agents: Vec::new(),
                confine: None,
                auto: false,
                team: Some(a2amx::messaging::TeamScope {
                    name: "demo".into(),
                    private: true,
                    allow: Vec::new(),
                    agents: Vec::new(),
                }),
            },
        ]
    );
}

#[test]
fn parse_resolves_agent_allow_only_for_the_declaring_session() {
    let sessions = team::parse(
        r#"team = "a2amx"
    allow_teams = ["other"]
    [[session]]
    name = "architect"
    command = ["cat"]
    allow_agents = ["saltmdb/architect"]
    [[session]]
    name = "developer"
    command = ["cat"]
    "#,
        std::path::Path::new("/work/demo"),
        false,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(sessions[0].team.as_ref().unwrap()).unwrap(),
        serde_json::json!({
            "name": "a2amx", "private": true, "allow": ["other"],
            "agents": ["saltmdb-architect"]
        })
    );
    assert_eq!(
        serde_json::to_value(sessions[1].team.as_ref().unwrap()).unwrap(),
        serde_json::json!({"name": "a2amx", "private": true, "allow": ["other"]})
    );
}
#[test]
fn parse_rejects_invalid_session_agent_allow_entries() {
    let invalid = [
        "architect",
        "/architect",
        "a/b/c",
        "saltmdb/",
        "a2amx/architect",
        "saltmdb/architect,saltmdb/architect",
    ];
    for entry in invalid {
        let allow = if entry.contains(',') {
            entry.replace(',', "\", \"")
        } else {
            entry.to_owned()
        };
        let text = format!(
            "team = \"a2amx\"\n[[session]]\nname = \"architect\"\ncommand = [\"cat\"]\nallow_agents = [\"{allow}\"]"
        );
        let error = team::parse(&text, std::path::Path::new("/work/demo"), false).expect_err(entry);
        assert!(error.to_string().contains("session"), "{entry}: {error}");
        assert!(error.to_string().contains("allow"), "{entry}: {error}");
    }
    let entries = (0..17)
        .map(|index| format!("\"saltmdb/agent{index}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let error = team::parse(&format!(
        "team = \"a2amx\"\n[[session]]\nname = \"architect\"\ncommand = [\"cat\"]\nallow_agents = [{entries}]"
    ), std::path::Path::new("/work/demo"), false)
    .expect_err("allow cap");
    assert!(error.to_string().contains("session"));
    assert!(error.to_string().contains("allow"));
    let error = team::parse("team = \"a2amx\"\nprivate = false\n[[session]]\nname = \"architect\"\ncommand = [\"cat\"]\nallow_agents = [\"saltmdb/architect\"]", std::path::Path::new("/work/demo"), false)
    .expect_err("public team");
    assert!(error.to_string().contains("session a2amx-architect"));
    assert!(error.to_string().contains("allow"));
    let error = team::parse("team = \"demo\"\nprivate = false\n[[session]]\nname = \"architect\"\ncommand = [\"cat\"]\nallow_agents = [\"saltmdb/architect\"]", std::path::Path::new("/work/demo"), false)
        .expect_err("public team");
    assert!(error.to_string().contains("session demo-architect"));
    assert!(error.to_string().contains("allow"));
}

#[test]
fn parse_defaults_name_to_role() {
    let sessions = team::parse(
        r#"
    [[session]]
    role = "architect"
    command = ["cat"]
    "#,
        std::path::Path::new("/work/demo"),
        false,
    )
    .unwrap();
    assert_eq!(sessions[0].name, "demo-architect");
    assert_eq!(sessions[0].role.as_deref(), Some("architect"));
}

#[test]
fn parse_defaults_role_name_before_prefix_and_reference_resolution() {
    let sessions = team::parse(
        r#"
    team = "t"
    [[session]]
    role = "architect"
    command = ["cat"]
    [[session]]
    name = "developer"
    command = ["cat"]
    watch = ["architect"]
    "#,
        std::path::Path::new("/work/demo"),
        false,
    )
    .unwrap();
    assert_eq!(sessions[0].name, "t-architect");
    assert_eq!(sessions[1].watch, ["t-architect"]);
}

#[test]
fn parse_explicit_name_wins_over_role() {
    let sessions = team::parse(
        r#"
    [[session]]
    name = "worker"
    role = "architect"
    command = ["cat"]
    "#,
        std::path::Path::new("/work/demo"),
        false,
    )
    .unwrap();
    assert_eq!(sessions[0].name, "demo-worker");
    assert_eq!(sessions[0].role.as_deref(), Some("architect"));
}

#[test]
fn parse_rejects_invalid_derived_names_with_name_guidance() {
    for (role, expected) in [
        (
            "Senior Reviewer",
            "session 1: name taken from role \"Senior Reviewer\"",
        ),
        ("s12", "session 1: name taken from role \"s12\""),
        (
            " architect",
            "session 1: name taken from role \" architect\"",
        ),
    ] {
        let text = format!("[[session]]\nrole = {role:?}\ncommand = [\"cat\"]");
        let error = team::parse(&text, std::path::Path::new("/work/demo"), false).expect_err(role);
        let error = error.to_string();
        assert!(error.contains(expected), "{error}");
        assert!(error.contains("set name explicitly"), "{error}");
    }
}

#[test]
fn parse_requires_name_or_role() {
    let error = team::parse(
        "[[session]]\ncommand = [\"cat\"]",
        std::path::Path::new("/work/demo"),
        false,
    )
    .expect_err("missing identity");
    assert_eq!(error.to_string(), "session 1: name or role is required");
}

#[test]
fn parse_rejects_duplicate_derived_names() {
    let error = team::parse(
        r#"
    [[session]]
    role = "developer"
    command = ["cat"]
    [[session]]
    role = "developer"
    command = ["cat"]
    "#,
        std::path::Path::new("/work/demo"),
        false,
    )
    .expect_err("duplicate role-only names");
    assert!(error.to_string().contains("duplicate name"), "{error:#}");
}

#[test]
fn parse_accepts_and_validates_roles() {
    let valid = team::parse(
        r#"
    [[session]]
    name = "worker"
    command = ["cat"]
    role = "architect"
    [[session]]
    name = "plain"
    command = ["cat"]
    "#,
        std::path::Path::new("/work/demo"),
        false,
    )
    .expect("valid roles");
    assert_eq!(valid[0].role.as_deref(), Some("architect"));
    assert_eq!(valid[1].role, None);

    for text in [
        "[[session]]\nname = \"worker\"\ncommand = [\"cat\"]\nrole = \"\"",
        &format!(
            "[[session]]\nname = \"worker\"\ncommand = [\"cat\"]\nrole = \"{}\"",
            "a".repeat(65)
        ),
        "[[session]]\nname = \"worker\"\ncommand = [\"cat\"]\nrole = \"a\\nb\"",
        "[[session]]\nname = \"worker\"\ncommand = [\"cat\"]\nrole = \" architect\"",
        "[[session]]\nname = \"worker\"\ncommand = [\"cat\"]\nrole = \"architect \"",
        "[[session]]\nname = \"worker\"\ncommand = [\"cat\"]\nrole = 5",
    ] {
        let error = team::parse(text, std::path::Path::new("/work/demo"), false).expect_err(text);
        assert!(format!("{error:#}").contains("role"), "{error:#}");
    }
}

#[test]
fn prefixed_role_errors_name_the_final_session() {
    let error = team::parse(
        r#"
    team = "team"
    [[session]]
    name = "worker"
    command = ["cat"]
    role = ""
    "#,
        std::path::Path::new("/work/demo"),
        false,
    )
    .expect_err("empty role");
    assert!(error.to_string().contains("session team-worker"));
}

#[test]
fn parse_prefixes_names_and_same_file_references() {
    let sessions = team::parse(
        r#"
    team = "a2amx"
    [[session]]
    name = "architect"
    command = ["claude"]
    watch = ["developer", "other/architect", "/hand"]
    control_from = ["developer"]
    [[session]]
    name = "developer"
    command = ["omp"]
    control_from = ["architect"]
    "#,
        std::path::Path::new("/work/demo"),
        false,
    )
    .unwrap();
    assert_eq!(sessions[0].name, "a2amx-architect");
    assert_eq!(sessions[1].name, "a2amx-developer");
    assert_eq!(
        sessions[0].watch,
        ["a2amx-developer", "other-architect", "hand"]
    );
    assert_eq!(sessions[0].control_from, ["a2amx-developer"]);
    assert_eq!(sessions[1].control_from, ["a2amx-architect"]);
}

#[test]
fn parse_rejects_unknown_local_and_malformed_explicit_references() {
    for key in ["watch", "control_from"] {
        for reference in ["outsider", "a/b/c", "/", "x/"] {
            let text = format!(
                "team = \"demo\"\n[[session]]\nname = \"architect\"\ncommand = [\"cat\"]\n{key} = [\"{reference}\"]"
            );
            let error =
                team::parse(&text, std::path::Path::new("/work/demo"), false).expect_err(&text);
            let error = format!("{error:#}");
            assert!(error.contains(key), "{error}");
            if reference == "outsider" {
                assert_eq!(
                    error,
                    format!("unknown session outsider in {key} of demo-architect")
                );
            }
        }
    }
    let sessions = team::parse("[[session]]\nname = \"architect\"\ncommand = [\"cat\"]\nwatch = [\"/outsider\", \"other/architect\", \"/hand\"]", std::path::Path::new("/work/demo"), false).unwrap();
    assert_eq!(sessions[0].watch, ["outsider", "other-architect", "hand"]);
}

#[test]
fn parse_rejects_invalid_team_keys_and_old_prefix() {
    for (keys, label) in [
        ("prefix = \"x\"", "prefix"),
        ("team = \"\"", "team"),
        ("team = \"demo-\"", "team"),
        ("team = 5", "team"),
        ("allow = [\"x\"]", "allow was renamed to allow_teams"),
        ("allow = []", "allow was renamed to allow_teams"),
        (
            "team = \"demo\"\nprivate = false\nallow_teams = [\"x\"]",
            "allow_teams needs a private team",
        ),
        (
            "team = \"demo\"\nallow_teams = [\"demo\"]",
            "allow_teams entry must not be the team itself",
        ),
        (
            "team = \"demo\"\nallow_teams = [\"x\", \"x\"]",
            "allow_teams entry",
        ),
        (
            "team = \"demo\"\nallow_teams = [\"Bad\"]",
            "allow_teams entry",
        ),
    ] {
        let text = format!("{keys}\n[[session]]\nname = \"architect\"\ncommand = [\"cat\"]");
        let error = team::parse(&text, std::path::Path::new("/work/demo"), false).expect_err(&text);
        assert!(format!("{error:#}").contains(label), "{error:#}");
    }
}

#[test]
fn parse_validates_prefixed_final_names() {
    for (prefix, final_name) in [
        ("Bad", "Bad"),
        (
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-architect",
        ),
    ] {
        let text =
            format!("team = \"{prefix}\"\n[[session]]\nname = \"architect\"\ncommand = [\"cat\"]");
        let error = team::parse(&text, std::path::Path::new("/work/demo"), false).expect_err(&text);
        assert!(format!("{error:#}").contains(final_name), "{error:#}");
    }
}

#[test]
fn parse_rejects_prefixed_self_watch() {
    let error = team::parse("team = \"a2amx\"\n[[session]]\nname = \"developer\"\ncommand = [\"cat\"]\nwatch = [\"developer\"]", std::path::Path::new("/work/demo"), false)
    .expect_err("prefixed self watch");
    assert!(error.to_string().contains("watches itself"), "{error:#}");
}

#[test]
fn parse_rejects_whitespace_in_executable_and_preserves_argument_boundaries() {
    for executable in ["claude --model opus", "/tmp/my executable", "cat\\t-n"] {
        let text = format!(
            "team = \"pfx\"\n[[session]]\nname = \"architect\"\ncommand = [\"{executable}\"]"
        );
        let error = team::parse(&text, std::path::Path::new("/work/demo"), false).expect_err(&text);
        let error = format!("{error:#}");
        assert!(
            error.contains("session pfx-architect: command[0]"),
            "{error}"
        );
    }
    let sessions = team::parse(
        "[[session]]\nname = \"architect\"\ncommand = [\"claude\", \"--model\", \"opus\"]",
        std::path::Path::new("/work/demo"),
        false,
    )
    .unwrap();
    assert_eq!(sessions[0].command, ["claude", "--model", "opus"]);
}

#[test]
fn parse_rejects_invalid_sessions_and_identifies_the_entry() {
    for (text, label) in [
        ("[[session]]\ncommand = [\"cat\"]", "session 1"),
        (
            "[[session]]\nname = \"Bad_Name\"\ncommand = [\"cat\"]",
            "Bad_Name",
        ),
        (
            "[[session]]\nname = \"a\"\ncommand = [\"cat\"]\n[[session]]\nname = \"a\"\ncommand = [\"cat\"]",
            "demo-a",
        ),
        ("[[session]]\nname = \"a\"\ncommand = []", "demo-a"),
        (
            "[[session]]\nname = \"a\"\ncommand = [\"cat\"]\nattach = true\n[[session]]\nname = \"b\"\ncommand = [\"cat\"]\nattach = true",
            "demo-b",
        ),
        (
            "[[session]]\nname = \"a\"\ncommand = [\"cat\"]\nextra = true",
            "session 1",
        ),
        ("[[session]]\nname = \"a\"\ncommand = \"cat\"", "session 1"),
    ] {
        let error = team::parse(text, std::path::Path::new("/work/demo"), false).expect_err(text);
        assert!(format!("{error:#}").contains(label), "{error:#}");
    }
    assert!(team::parse("", Path::new("/work/proj"), false).is_err());
    assert!(team::parse("unexpected = true", Path::new("/work/proj"), false).is_err());
}

#[test]
fn parse_validates_watch_names_and_self_reference() {
    let parsed = team::parse(
        r#"
    [[session]]
    name = "architect"
    command = ["cat"]
    watch = ["/developer"]
    "#,
        std::path::Path::new("/work/demo"),
        false,
    )
    .unwrap();
    assert_eq!(parsed[0].watch, vec!["developer"]);
    for (text, label) in [
        (
            "[[session]]\nname = \"self\"\ncommand = [\"cat\"]\nwatch = [\"self\"]",
            "watches itself",
        ),
        (
            "[[session]]\nname = \"self\"\ncommand = [\"cat\"]\nwatch = [\"Bad_Name\"]",
            "unknown session Bad_Name in watch of demo-self",
        ),
    ] {
        let error = team::parse(text, std::path::Path::new("/work/demo"), false).expect_err(text);
        assert!(format!("{error:#}").contains(label), "{error:#}");
    }
}

#[test]
fn parse_prefixes_daemon_like_session_name() {
    let sessions = team::parse(
        "[[session]]\nname = \"a2amx-daemon\"\ncommand = [\"cat\"]",
        std::path::Path::new("/work/demo"),
        false,
    )
    .expect("prefixed daemon-like session");
    assert_eq!(sessions[0].name, "demo-a2amx-daemon");
}
#[test]
fn flag_sessions_support_bare_executables_and_first_equals_only() {
    assert_eq!(
        team::flag_sessions(&["architect=claude".into(), "developer=omp".into()]).unwrap(),
        vec![
            TeamSession {
                name: "architect".into(),
                command: vec!["claude".into()],
                cwd: None,
                attach: true,
                reset: None,
                control_from: Vec::new(),
                watch: Vec::new(),
                heartbeat: None,
                activity: None,
                role: None,
                remote: None,
                allow_agents: Vec::new(),
                confine: None,
                auto: false,
                team: None,
            },
            TeamSession {
                name: "developer".into(),
                command: vec!["omp".into()],
                cwd: None,
                attach: false,
                reset: None,
                control_from: Vec::new(),
                watch: Vec::new(),
                heartbeat: None,
                activity: None,
                role: None,
                remote: None,
                allow_agents: Vec::new(),
                confine: None,
                auto: false,
                team: None,
            },
        ]
    );
    assert_eq!(
        team::flag_sessions(&["a=b=c".into()]).unwrap()[0].command,
        ["b=c"]
    );
    for specs in [
        vec!["architect=claude -p".into()],
        vec!["architect".into()],
        vec!["=claude".into()],
        vec!["architect=".into()],
        vec!["Bad_Name=cat".into()],
        vec!["a=cat".into(), "a=cat".into()],
        vec![],
    ] {
        assert!(team::flag_sessions(&specs).is_err(), "{specs:?}");
    }
    let error = team::flag_sessions(&["a=cat -n".into()]).unwrap_err();
    assert!(error.to_string().contains("team file"));
}

#[test]
fn confinement_team_defaults_and_validation_are_explicit() {
    let sessions = team::parse(
        r#"
    [confine]
    enabled = true
    [[session]]
    name = "worker"
    command = ["cat"]
    [session.confine]
    write = ["./data"]
    hide = ["./secrets"]
    unhide = ["./public"]
    grants = ["docker"]
    "#,
        std::path::Path::new("/work/demo"),
        false,
    )
    .unwrap();
    let request = sessions[0].confine.as_ref().expect("confined");
    assert_eq!(request.write, ["./data"]);
    assert!(request.grants.contains(&"docker".to_owned()));
    let sessions = team::parse(
        r#"
    [[session]]
    name = "worker"
    command = ["cat"]
    [session.confine]
    hide = ["./secrets"]
    "#,
        std::path::Path::new("/work/demo"),
        false,
    )
    .expect("options without enabled");
    assert!(sessions[0].confine.is_some());
    let sessions = team::parse(
        r#"
    [[session]]
    name = "worker"
    command = ["codex"]
    [session.confine]
    enabled = true
    "#,
        std::path::Path::new("/work/demo"),
        false,
    )
    .expect("codex confinement");
    assert!(sessions[0].confine.is_some());
}
#[test]
fn confined_claude_prompt_file_after_separator_is_ignored() {
    let sessions = team::parse(
        r#"
    [confine]
    enabled = true
    [[session]]
    name = "worker"
    command = ["claude", "--", "--append-system-prompt-file=prompt.txt"]
    "#,
        std::path::Path::new("/work/demo"),
        false,
    )
    .expect("post-separator prompt option is not part of the command");
    assert!(sessions[0].confine.is_some());
}
