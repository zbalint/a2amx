use super::*;

#[test]
fn heartbeat_team_entry_requires_watch_and_validates_interval() {
    let parsed = team::parse(
        r#"
    [[session]]
    name = "architect"
    command = ["sh"]
    watch = ["/developer"]
    heartbeat = "30m"
    "#,
        std::path::Path::new("/work/demo"),
        false,
    )
    .unwrap();
    assert_eq!(parsed[0].heartbeat.as_deref(), Some("30m"));

    let no_watch = team::parse(
        r#"
    [[session]]
    name = "architect"
    command = ["sh"]
    heartbeat = "30m"
    "#,
        std::path::Path::new("/work/demo"),
        false,
    )
    .unwrap_err();
    assert!(format!("{no_watch:#}").contains("heartbeat needs a non-empty watch"));

    for value in ["0s", "25h", "30", "m", "1d", "-5m"] {
        let error = team::parse(&format!(
            "[[session]]\nname = \"architect\"\ncommand = [\"sh\"]\nwatch = [\"/developer\"]\nheartbeat = {value:?}"
        ), std::path::Path::new("/work/demo"), false)
        .unwrap_err();
        assert!(
            format!("{error:#}").contains("invalid heartbeat"),
            "{value}: {error:#}"
        );
    }
}

#[test]
fn activity_team_entry_requires_watch_duration_and_validates_fields() {
    let parsed = team::parse(
        r#"
    [[session]]
    name = "architect"
    command = ["sh"]
    watch = ["/developer"]
    activity = { working_after = "2m", idle_after = "30s" }
    "#,
        std::path::Path::new("/work/demo"),
        false,
    )
    .unwrap();
    assert_eq!(
        parsed[0]
            .activity
            .as_ref()
            .and_then(|activity| activity.working_after.as_deref()),
        Some("2m")
    );

    for (text, expected) in [
        (
            "[[session]]\nname = \"architect\"\ncommand = [\"sh\"]\nactivity = { idle_after = \"30s\" }",
            "activity needs a non-empty watch",
        ),
        (
            "[[session]]\nname = \"architect\"\ncommand = [\"sh\"]\nwatch = [\"/developer\"]\nactivity = {}",
            "activity needs at least one duration",
        ),
        (
            "[[session]]\nname = \"architect\"\ncommand = [\"sh\"]\nwatch = [\"/developer\"]\nactivity = { idle_after = \"25h\" }",
            "invalid activity idle_after",
        ),
        (
            "[[session]]\nname = \"architect\"\ncommand = [\"sh\"]\nwatch = [\"/developer\"]\nactivity = { nope = \"30s\" }",
            "unknown field",
        ),
    ] {
        let error = team::parse(text, std::path::Path::new("/work/demo"), false).unwrap_err();
        assert!(
            format!("{error:#}").contains(expected),
            "{expected}: {error:#}"
        );
    }
}

#[test]
fn activity_team_cycles_are_rejected_in_file_order() {
    let mutual = team::parse(
        r#"
    [[session]]
    name = "a"
    command = ["sh"]
    watch = ["b"]
    activity = { idle_after = "30s" }
    [[session]]
    name = "b"
    command = ["sh"]
    watch = ["a"]
    activity = { working_after = "2m" }
    "#,
        std::path::Path::new("/work/demo"),
        false,
    )
    .unwrap_err();
    assert!(
        format!("{mutual:#}").contains("activity notices would loop between demo-a and demo-b")
    );

    let ring = team::parse(
        r#"
    [[session]]
    name = "a"
    command = ["sh"]
    watch = ["b"]
    activity = { idle_after = "30s" }
    [[session]]
    name = "b"
    command = ["sh"]
    watch = ["c"]
    activity = { idle_after = "30s" }
    [[session]]
    name = "c"
    command = ["sh"]
    watch = ["a"]
    activity = { idle_after = "30s" }
    "#,
        std::path::Path::new("/work/demo"),
        false,
    )
    .unwrap_err();
    assert!(
        format!("{ring:#}")
            .contains("activity notices would loop between demo-a, demo-b and demo-c")
    );

    team::parse(
        r#"
    [[session]]
    name = "a"
    command = ["sh"]
    watch = ["b"]
    activity = { idle_after = "30s" }
    [[session]]
    name = "b"
    command = ["sh"]
    "#,
        std::path::Path::new("/work/demo"),
        false,
    )
    .unwrap();
}
#[test]
fn confine_table_merges_team_defaults_and_session_grants() {
    let sessions = team::parse(
        r#"
    team = "myproject"
    [confine]
    enabled = true
    write = ["~/.ctrscm"]
    default_hides = true
    [[session]]
    name = "architect"
    command = ["claude"]
    [session.confine]
    grants = ["docker"]
    keep_env = ["GITHUB_TOKEN", "GITHUB_TOKEN"]
    write = ["~/.cache"]
    [[session]]
    name = "scratch"
    command = ["omp"]
    [session.confine]
    enabled = false
    "#,
        std::path::Path::new("/work/demo"),
        false,
    )
    .expect("new confine table");
    assert_eq!(
        sessions[0].confine,
        Some(a2amx::confine::ConfineRequest {
            write: vec!["~/.ctrscm".into(), "~/.cache".into()],
            hide: vec![],
            unhide: vec![],
            no_default_hides: false,
            grants: vec!["docker".into()],
            auto: false,
            reasons: vec![],
            keep_env: vec!["GITHUB_TOKEN".into()],
            team_file: None,
        })
    );
    assert_eq!(sessions[1].confine, None);
}

#[test]
fn confine_table_merges_team_grants_before_session_grants() {
    let sessions = team::parse(
        r#"
    [confine]
    enabled = true
    grants = ["cargo-token"]
    [[session]]
    name = "with-own"
    command = ["cat"]
    [session.confine]
    grants = ["go"]
    [[session]]
    name = "without-own"
    command = ["cat"]
    "#,
        std::path::Path::new("/work/demo"),
        false,
    )
    .expect("team grants");
    assert_eq!(
        sessions[0]
            .confine
            .as_ref()
            .expect("with-own confinement")
            .grants,
        vec!["cargo-token", "go"]
    );
    assert_eq!(
        sessions[1]
            .confine
            .as_ref()
            .expect("without-own confinement")
            .grants,
        vec!["cargo-token"]
    );
}

#[test]
fn confine_table_rejects_old_keys_and_team_grants() {
    let old_inputs = [
        (
            "confine = false\n[[session]]\nname = 'worker'\ncommand = ['cat']",
            "ConfineTable",
        ),
        (
            "confine = true\n[[session]]\nname = 'worker'\ncommand = ['cat']",
            "ConfineTable",
        ),
        (
            "[[session]]\nname = 'worker'\ncommand = ['cat']\nconfine = false",
            "ConfineTable",
        ),
        (
            "[[session]]\nname = 'worker'\ncommand = ['cat']\nconfine_write = ['x']",
            "confine_write",
        ),
        (
            "[[session]]\nname = 'worker'\ncommand = ['cat']\nconfine_hide = ['x']",
            "confine_hide",
        ),
        (
            "[[session]]\nname = 'worker'\ncommand = ['cat']\nconfine_unhide = ['x']",
            "confine_unhide",
        ),
        (
            "[[session]]\nname = 'worker'\ncommand = ['cat']\nconfine_no_default_hides = true",
            "confine_no_default_hides",
        ),
        (
            "[[session]]\nname = 'worker'\ncommand = ['cat']\nconfine_allow_containers = true",
            "confine_allow_containers",
        ),
    ];
    for (text, expected) in old_inputs {
        let error = team::parse(text, std::path::Path::new("/work/demo"), false)
            .expect_err("old confinement syntax");
        let full = format!("{error:#}");
        assert!(full.contains(expected), "{full}");
        assert!(
            !full.contains("replaced") && !full.contains("renamed"),
            "{full}"
        );
    }

    team::parse("[confine]\ngrants = ['gh', 'ssh', 'git-credentials', 'docker']\n[[session]]\nname = 'worker'\ncommand = ['cat']", std::path::Path::new("/work/demo"), false)
    .expect("credential team grants");
    for grant in ["podman", "harness-config"] {
        let error = team::parse(
            &format!(
                "[confine]\ngrants = ['{grant}']\n[[session]]\nname = 'worker'\ncommand = ['cat']"
            ),
            std::path::Path::new("/work/demo"),
            false,
        )
        .expect_err("non-credential team grant");
        assert!(error.to_string().contains("grant it in the session"));
    }
}

#[test]
fn confine_table_keeps_credentials_per_session_only() {
    let team_error = team::parse(
        "[confine]\nkeep_env = ['GITHUB_TOKEN']\n[[session]]\nname = 'worker'\ncommand = ['cat']",
        std::path::Path::new("/work/demo"),
        false,
    )
    .expect_err("team keep_env");
    assert_eq!(
        team_error.to_string(),
        "[confine] keep_env: a credential variable is returned per session, not team-wide; put keep_env in [session.confine]"
    );

    let sessions = team::parse("[[session]]\nname = 'worker'\ncommand = ['cat']\n[session.confine]\nkeep_env = ['GITHUB_TOKEN']", std::path::Path::new("/work/demo"), false)
        .expect("keep_env without enabled");
    assert_eq!(
        sessions[0].confine.as_ref().unwrap().keep_env,
        ["GITHUB_TOKEN"]
    );

    let sessions = team::parse("[[session]]\nname = 'worker'\ncommand = ['cat']\n[session.confine]\nenabled = true\nkeep_env = ['GITHUB_TOKEN', 'GITHUB_TOKEN']", std::path::Path::new("/work/demo"), false)
    .expect("session keep_env");
    assert_eq!(
        sessions[0]
            .confine
            .as_ref()
            .expect("confined session")
            .keep_env,
        ["GITHUB_TOKEN"]
    );
}

#[test]
fn confine_entry_syntax_is_checked_at_parse_time() {
    let session_error = team::parse(
        r#"
    [[session]]
    name = "worker"
    command = ["cat"]
    [session.confine]
    write = ["$HOME/.cache"]
    "#,
        std::path::Path::new("/work/demo"),
        false,
    )
    .expect_err("session shell-style path");
    assert!(
        session_error
            .to_string()
            .contains("session worker: write entry \"$HOME/.cache\": environment variables are not expanded; write the path out, or use ~/ for your home")
    );

    let team_error = team::parse(
        r#"
    [confine]
    write = ["$HOME/.cache"]
    [[session]]
    name = "worker"
    command = ["cat"]
    "#,
        std::path::Path::new("/work/demo"),
        false,
    )
    .expect_err("team shell-style path");
    assert!(
        team_error
            .to_string()
            .contains("[confine] write entry \"$HOME/.cache\": environment variables are not expanded; write the path out, or use ~/ for your home")
    );
}

#[test]
fn confine_auto_is_accepted_in_any_team_file_and_team_false_is_accepted() {
    for (text, dir, global) in [
        (
            "[confine]\nenabled = true\nauto = true\n[[session]]\nname = 'worker'\ncommand = ['cat']",
            Path::new("/work/project"),
            true,
        ),
        (
            "[confine]\nenabled = true\nauto = true\n[[session]]\nname = 'worker'\ncommand = ['cat']",
            Path::new("/work/demo"),
            false,
        ),
    ] {
        let sessions = team::parse(text, dir, global).expect("auto in team file");
        assert!(sessions[0].auto);
    }

    for value in ["true", "false"] {
        let error = team::parse(&format!(
            "[confine]\nenabled = true\n[[session]]\nname = 'worker'\ncommand = ['cat']\n[session.confine]\nauto = {value}"
        ), std::path::Path::new("/work/demo"), false)
        .expect_err("session auto");
        assert_eq!(
            error.to_string(),
            "session worker: auto belongs in the team's [confine] table, not [session.confine]"
        );
    }

    let missing_enabled = team::parse(
        "[confine]\nauto = true\n[[session]]\nname = 'worker'\ncommand = ['cat']",
        Path::new("/work/project"),
        true,
    )
    .expect("auto defaults to confinement");
    assert!(missing_enabled[0].auto);

    let global_false = team::parse(
        "[confine]\nauto = false\n[[session]]\nname = 'worker'\ncommand = ['cat']",
        Path::new("/work/project"),
        true,
    )
    .expect("global false");
    assert!(!global_false[0].auto);

    let local_false = team::parse(
        "[confine]\nauto = false\n[[session]]\nname = 'worker'\ncommand = ['cat']",
        std::path::Path::new("/work/demo"),
        false,
    )
    .expect("local false");
    assert!(!local_false[0].auto);
    let auto_disabled = team::parse(
        "[confine]\nenabled = false\nauto = true\n[[session]]\nname = 'worker'\ncommand = ['cat']",
        Path::new("/work/project"),
        true,
    )
    .expect_err("auto disabled");
    assert_eq!(
        auto_disabled.to_string(),
        "[confine] auto needs confinement; remove enabled = false"
    );

    let disabled = team::parse(
        "[confine]\nenabled = false\n[[session]]\nname = 'worker'\ncommand = ['cat']",
        Path::new("/work/project"),
        true,
    )
    .expect("disabled confinement");
    assert_eq!(disabled[0].confine, None);

    let mixed = team::parse(
        "[confine]\n[[session]]\nname = 'disabled'\ncommand = ['cat']\n[session.confine]\nenabled = false\n[[session]]\nname = 'enabled'\ncommand = ['cat']",
        Path::new("/work/project"),
        true,
    )
    .expect("mixed session confinement");
    assert_eq!(mixed[0].confine, None);
    assert!(mixed[1].confine.is_some());
}

#[test]
fn confine_table_requires_enabled_for_own_options_and_deduplicates_lists() {
    let sessions = team::parse(
        "[[session]]\nname = 'worker'\ncommand = ['cat']\n[session.confine]\nwrite = ['x']",
        std::path::Path::new("/work/demo"),
        false,
    )
    .expect("options default to enabled");
    assert!(sessions[0].confine.is_some());

    let sessions = team::parse("[confine]\nenabled = true\nwrite = ['x', 'x']\n[[session]]\nname = 'worker'\ncommand = ['cat']\n[session.confine]\nwrite = ['x', 'y', 'y']\ngrants = ['docker', 'docker']", std::path::Path::new("/work/demo"), false)
    .expect("deduplicated confine lists");
    let request = sessions[0].confine.as_ref().expect("confined");
    assert_eq!(request.write, ["x", "y"]);
    assert_eq!(request.grants, ["docker"]);
}

#[test]
fn confine_table_covers_disabled_and_merged_limit_rules() {
    let inherited = team::parse("[confine]\nenabled = false\nwrite = ['team']\n[[session]]\nname = 'worker'\ncommand = ['cat']\n[session.confine]\nenabled = true", std::path::Path::new("/work/demo"), false)
    .expect("session enables team defaults");
    assert_eq!(
        inherited[0].confine.as_ref().expect("confined").write,
        ["team"]
    );
    let team_disabled = team::parse(
        "[confine]\nenabled = false\nwrite = ['team']\n[[session]]\nname = 'worker'\ncommand = ['cat']\n[session.confine]\nwrite = ['own']",
        std::path::Path::new("/work/demo"),
        false,
    )
    .expect("session options override team disabled");
    assert_eq!(
        team_disabled[0]
            .confine
            .as_ref()
            .expect("session confinement")
            .write,
        ["team", "own"]
    );
    let default_hides = team::parse(
        "[confine]\nenabled = true\n[[session]]\nname = 'worker'\ncommand = ['cat']",
        std::path::Path::new("/work/demo"),
        false,
    )
    .expect("default hides on");
    assert!(
        !default_hides[0]
            .confine
            .as_ref()
            .expect("confined")
            .no_default_hides
    );

    let enabled = team::parse(
        "[confine]\nwrite = ['team']\n[[session]]\nname = 'worker'\ncommand = ['cat']\n",
        std::path::Path::new("/work/demo"),
        false,
    )
    .expect("team options default to enabled");
    assert!(enabled[0].confine.is_some());
    let no_table = team::parse(
        "[[session]]\nname = 'worker'\ncommand = ['cat']",
        std::path::Path::new("/work/demo"),
        false,
    )
    .expect("no confine table");
    assert_eq!(no_table[0].confine, None);

    let disabled = team::parse("[confine]\nenabled = true\nwrite = ['team']\n[[session]]\nname = 'worker'\ncommand = ['cat']\n[session.confine]\nenabled = false", std::path::Path::new("/work/demo"), false)
    .expect("session can opt out");
    assert_eq!(disabled[0].confine, None);

    let own_option = team::parse("[confine]\nenabled = true\n[[session]]\nname = 'worker'\ncommand = ['cat']\n[session.confine]\nenabled = false\nwrite = ['own']", std::path::Path::new("/work/demo"), false)
        .expect_err("disabled session options");
    assert!(
        own_option
            .to_string()
            .contains("options are set but enabled = false")
    );

    let unknown_key = team::parse(
        "[confine]\nenabled = true\nnope = true\n[[session]]\nname = 'worker'\ncommand = ['cat']",
        std::path::Path::new("/work/demo"),
        false,
    )
    .expect_err("unknown confine key");
    assert!(format!("{unknown_key:#}").contains("unknown field"));

    let unknown_grant = team::parse("[[session]]\nname = 'worker'\ncommand = ['cat']\n[session.confine]\nenabled = true\ngrants = ['container']", std::path::Path::new("/work/demo"), false)
    .expect_err("unknown session grant");
    assert!(
        unknown_grant
            .to_string()
            .contains("closest match: containerd")
    );

    let team_paths = (0..40)
        .map(|index| format!("'team-{index}'"))
        .collect::<Vec<_>>()
        .join(", ");
    let session_paths = (0..30)
        .map(|index| format!("'session-{index}'"))
        .collect::<Vec<_>>()
        .join(", ");
    let merged_limit = team::parse(&format!(
        "[confine]\nenabled = true\nwrite = [{team_paths}]\n[[session]]\nname = 'worker'\ncommand = ['cat']\n[session.confine]\nwrite = [{session_paths}]"
    ), std::path::Path::new("/work/demo"), false)
    .expect_err("merged write limit");
    assert!(
        merged_limit
            .to_string()
            .contains("session worker: write has at most 64 entries")
    );
    let long_path = "x".repeat(4097);
    let merged_bytes = team::parse(&format!(
        "[confine]\nenabled = true\nwrite = ['team']\n[[session]]\nname = 'worker'\ncommand = ['cat']\n[session.confine]\nwrite = ['{long_path}']"
    ), std::path::Path::new("/work/demo"), false)
    .expect_err("merged write byte limit");
    assert!(
        merged_bytes
            .to_string()
            .contains("write entry exceeds 4096 bytes")
    );
}
