use super::*;

#[test]
fn global_team_name_is_normalized_valid_and_bounded() {
    assert_eq!(
        team::team_name_for(Path::new("/work/My_Project.v2"), "R").unwrap(),
        "my-project-v2"
    );
    assert_eq!(
        team::team_name_for(Path::new("/work/ABC"), "R").unwrap(),
        "abc"
    );
    for path in ["/work/s12", "/work/---"] {
        let error = team::team_name_for(Path::new(path), "R")
            .unwrap_err()
            .to_string();
        assert!(error.contains(path));
        assert!(error.ends_with("; R"), "{error}");
    }
    let error = team::team_name_for(Path::new("/"), "R")
        .unwrap_err()
        .to_string();
    assert!(error.ends_with("; R"), "{error}");
    let path = format!("/work/{}-{}", "a".repeat(39), "b".repeat(30));
    assert_eq!(
        team::team_name_for(Path::new(&path), "R").unwrap(),
        "a".repeat(39)
    );
    let bounded = team::team_name_for(Path::new(&path), "R").unwrap();
    let sessions = team::parse(
        "[[session]]\nname = 'architect'\ncommand = ['cat']\n[[session]]\nname = 'consultant'\ncommand = ['cat']",
        Path::new(&path),
        false,
    )
    .unwrap();
    assert_eq!(sessions[0].name, format!("{bounded}-architect"));
    assert_eq!(sessions[1].name, format!("{bounded}-consultant"));
}

#[test]
fn remote_modes_parse_and_require_confinement() {
    let sessions = team::parse(
        r#"
    [[session]]
    name = "both"
    command = ["cat"]
    remote = "both"
    [session.confine]
    enabled = true
    [[session]]
    name = "send"
    command = ["cat"]
    remote = "send"
    [session.confine]
    enabled = true
    "#,
        std::path::Path::new("/work/demo"),
        false,
    )
    .unwrap();
    assert_eq!(sessions[0].remote, Some(RemoteMode::Both));
    assert_eq!(sessions[1].remote, Some(RemoteMode::Send));

    let error = team::parse(
        r#"
    [[session]]
    name = "worker"
    command = ["cat"]
    remote = "maybe"
    [session.confine]
    enabled = true
    "#,
        std::path::Path::new("/work/demo"),
        false,
    )
    .unwrap_err();
    let error = format!("{error:#}");
    assert!(error.contains("send"));
    assert!(error.contains("both"));

    let error = team::parse(
        r#"
    [[session]]
    name = "worker"
    command = ["cat"]
    remote = "both"
    "#,
        std::path::Path::new("/work/demo"),
        false,
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("remote messaging needs a confined session"));
}

#[test]
fn global_guard_rejects_protected_directories_in_order() {
    let home = Path::new("/home/u");
    let state = Path::new("/home/u/.a2amx");
    for path in [
        "/home",
        "/home/u",
        "/home/other",
        "/home/u/.ssh",
        "/home/u/.a2amx/x",
        "/etc",
        "/etc/x",
        "/usr/local",
        "/tmp",
        "/var",
        "/var/log/x",
        "/mnt/c",
        "/mnt/c/Windows",
        "/mnt/C/users/x",
        "/MNT/C/Windows",
        "/Mnt/c/Users/x",
        "/MNT/WSL",
    ] {
        assert!(
            team::check_global_allowed(Path::new(path), Some(home), state).is_err(),
            "{path}"
        );
    }
    for path in [
        "/home/u/work/p",
        "/home/u/Documents/p",
        "/tmp/p",
        "/var/www/p",
        "/mnt/c/p",
        "/mnt/c/Users/x/p",
        "/srv/p",
    ] {
        assert!(
            team::check_global_allowed(Path::new(path), Some(home), state).is_ok(),
            "{path}"
        );
    }
    assert!(team::check_global_allowed(Path::new("/"), None, state).is_err());
    assert!(team::check_global_allowed(Path::new("/etc"), None, state).is_err());
    assert!(team::check_global_allowed(Path::new("/work/p"), None, state).is_ok());
    assert!(
        team::check_global_allowed(Path::new("/"), Some(home), state)
            .unwrap_err()
            .to_string()
            .contains("filesystem root")
    );
    assert!(
        team::check_global_allowed(Path::new("/home/u"), Some(home), state)
            .unwrap_err()
            .to_string()
            .contains("home directory")
    );
    assert!(
        team::check_global_allowed(Path::new("/home/u/.a2amx/x"), Some(home), state)
            .unwrap_err()
            .to_string()
            .contains("hidden directory")
    );
}

#[test]
fn new_confine_options_require_confine_before_daemon_connection() {
    let state = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let user_home = tempfile::tempdir().unwrap();
    let runtime_dir = tempfile::tempdir().unwrap();

    for (option, value) in [
        ("--confine-keep-env", "GITHUB_TOKEN"),
        ("--confine-grant", "docker"),
    ] {
        let args = ["new", option, value, "--", "true"];
        let output = run_binary_in_with_home_and_runtime(
            state.path(),
            project.path(),
            user_home.path(),
            runtime_dir.path(),
            &args,
        )
        .unwrap();
        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("--confine-* flags require --confine"),
            "{option}: {stderr}"
        );
        assert!(
            !stderr.contains("cannot reach the a2amx daemon"),
            "{option}: guard did not run before daemon connection: {stderr}"
        );
    }
}

#[test]
fn parse_omitted_team_derives_private_team_from_directory() {
    let sessions = team::parse(
        "[[session]]\nname = 'a'\ncommand = ['cat']",
        Path::new("/work/proj"),
        false,
    )
    .expect("derived team");
    assert_eq!(sessions[0].name, "proj-a");
    let scope = sessions[0].team.as_ref().expect("private team");
    assert_eq!(scope.name, "proj");
    assert!(scope.private);
}

#[test]
fn parse_derives_private_team_and_rejects_global_team_key() {
    let text = r#"
[[session]]
name = "architect"
command = ["cat"]
[[session]]
name = "consultant"
command = ["cat"]
"#;
    let sessions = team::parse(text, Path::new("/work/proj"), false).unwrap();
    assert_eq!(
        sessions
            .iter()
            .map(|session| session.name.as_str())
            .collect::<Vec<_>>(),
        ["proj-architect", "proj-consultant"]
    );
    assert!(
        sessions
            .iter()
            .all(|session| session.team.as_ref().is_some_and(|team| team.private))
    );
    let error = team::parse(
        "team = 'wrong'\n[[session]]\nname = 'a'\ncommand = ['cat']",
        Path::new("/work/proj"),
        true,
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("must not set team"));
}
#[test]
fn parse_explicit_team_works_at_filesystem_root() {
    let sessions = team::parse(
        "team = 'root-team'\n[[session]]\nname = 'worker'\ncommand = ['cat']",
        Path::new("/"),
        false,
    )
    .expect("explicit team at root");
    assert_eq!(sessions[0].name, "root-team-worker");
    assert_eq!(
        sessions[0].team.as_ref().expect("team scope").name,
        "root-team"
    );
}
#[test]
fn global_guard_characterizes_root_reason() {
    let error = team::check_global_allowed(Path::new("/"), None, Path::new("/state"))
        .unwrap_err()
        .to_string();
    assert!(error.contains(": the filesystem root;"), "{error}");
}

#[test]
fn global_guard_home_precedes_container_reason() {
    let error = team::check_global_allowed(
        Path::new("/home"),
        Some(Path::new("/home/u")),
        Path::new("/home/u/.a2amx"),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains(": your home directory;"), "{error}");
}

#[test]
fn global_guard_hidden_home_precedes_state_reason() {
    let error = team::check_global_allowed(
        Path::new("/home/u/.a2amx"),
        Some(Path::new("/home/u")),
        Path::new("/home/u/.a2amx"),
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.contains(": a hidden directory under your home directory;"),
        "{error}"
    );
}

#[test]
fn global_guard_characterizes_user_data_reason() {
    let error = team::check_global_allowed(
        Path::new("/home/u/Documents"),
        Some(Path::new("/home/u")),
        Path::new("/state"),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains(": a user-data folder;"), "{error}");
}

#[test]
fn global_guard_characterizes_system_reason() {
    for path in ["/state/project", "/etc/x", "/MNT/WSL", "/MNT/C/Windows"] {
        let error = team::check_global_allowed(Path::new(path), None, Path::new("/state"))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(": the a2amx directory or a system directory;"),
            "{error}"
        );
    }
}

#[test]
fn global_guard_characterizes_container_reason() {
    for path in ["/tmp", "/mnt/c"] {
        let error = team::check_global_allowed(Path::new(path), None, Path::new("/state"))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(": a top-level container directory;"),
            "{error}"
        );
    }
}

#[test]
fn global_guard_characterizes_other_home_reason() {
    let error = team::check_global_allowed(
        Path::new("/home/other"),
        Some(Path::new("/home/u")),
        Path::new("/state"),
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.contains(": another user's home directory;"),
        "{error}"
    );
}

#[test]
fn global_guard_characterizes_windows_home_reason() {
    let error = team::check_global_allowed(Path::new("/Mnt/c/Users/u"), None, Path::new("/state"))
        .unwrap_err()
        .to_string();
    assert!(error.contains(": a Windows home directory;"), "{error}");
}

#[test]
fn parse_reports_invalid_team_name_with_team_prefix() {
    let error = team::parse(
        "team = 'bad name!'",
        std::path::Path::new("/work/demo"),
        false,
    )
    .unwrap_err()
    .to_string();
    assert!(error.starts_with("team \"bad name!\": "), "{error}");
}

#[test]
fn parse_reports_invalid_allow_entry_with_entry_prefix() {
    let error = team::parse(
        "team = 'demo'\nallow_teams = ['bad name!']",
        std::path::Path::new("/work/demo"),
        false,
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.starts_with("allow_teams entry \"bad name!\": "),
        "{error}"
    );
}

#[test]
fn parse_prefix_is_rejected_by_strict_parser() {
    let error = team::parse(
        "prefix = 'old'\nprivate = true",
        std::path::Path::new("/work/demo"),
        false,
    )
    .unwrap_err();
    let full = format!("{error:#}");
    assert!(full.contains("prefix"), "{full}");
    assert!(!full.contains("renamed"), "{full}");
}

#[test]
fn parse_private_defaults_when_team_is_omitted() {
    let sessions = team::parse(
        "private = true\n[[session]]\nname = 'a'\ncommand = ['cat']",
        Path::new("/work/proj"),
        false,
    )
    .unwrap();
    assert_eq!(sessions[0].name, "proj-a");
    assert!(sessions[0].team.as_ref().is_some_and(|team| team.private));
}

#[test]
fn parse_characterizes_allow_requires_team() {
    assert_eq!(
        team::parse("allow = []", Path::new("/work/proj"), false)
            .unwrap_err()
            .to_string(),
        "allow was renamed to allow_teams"
    );
}
#[test]
fn parse_rejects_session_two_legacy_allow_with_index() {
    let error = team::parse(
        "[[session]]\nname = 'a'\ncommand = ['cat']\n[[session]]\nname = 'b'\ncommand = ['cat']\nallow = ['x/y']",
        Path::new("/work/proj"),
        false,
    )
    .expect_err("legacy session allow");
    assert_eq!(
        error.to_string(),
        "session 2: allow was renamed to allow_agents"
    );
}

#[test]
fn parse_characterizes_empty_team() {
    assert_eq!(
        team::parse("team = ''", Path::new("/work/proj"), false)
            .unwrap_err()
            .to_string(),
        "team must not be empty"
    );
}

#[test]
fn parse_characterizes_team_trailing_dash() {
    assert_eq!(
        team::parse("team = 'demo-'", Path::new("/work/proj"), false)
            .unwrap_err()
            .to_string(),
        "team \"demo-\" must not end in '-'; the dash is added for you"
    );
}

#[test]
fn parse_public_team_precedes_allow_validation() {
    assert_eq!(
        team::parse(
            "team = 'demo'\nprivate = false\nallow_teams = ['demo', 'demo']",
            Path::new("/work/proj"),
            false,
        )
        .unwrap_err()
        .to_string(),
        "allow_teams needs a private team"
    );
}

#[test]
fn parse_characterizes_self_allow() {
    assert_eq!(
        team::parse(
            "team = 'demo'\nallow_teams = ['demo']",
            Path::new("/work/proj"),
            false
        )
        .unwrap_err()
        .to_string(),
        "allow_teams entry must not be the team itself"
    );
}

#[test]
fn parse_characterizes_repeated_allow() {
    assert_eq!(
        team::parse(
            "team = 'demo'\nallow_teams = ['other', 'other']",
            Path::new("/work/proj"),
            false
        )
        .unwrap_err()
        .to_string(),
        "allow_teams entry \"other\" is repeated"
    );
}

#[test]
fn parse_characterizes_role_name_error() {
    let error = team::parse(
        "[[session]]\nrole = 'Bad'\ncommand = ['cat']",
        std::path::Path::new("/work/demo"),
        false,
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("session 1: name taken from role \"Bad\":"),
        "{error}"
    );
    assert!(error.ends_with("; set name explicitly"), "{error}");
}

#[test]
fn parse_characterizes_missing_name_and_role() {
    assert_eq!(
        team::parse(
            "[[session]]\ncommand = ['cat']",
            Path::new("/work/proj"),
            false
        )
        .unwrap_err()
        .to_string(),
        "session 1: name or role is required"
    );
}

#[test]
fn parse_characterizes_session_allow_without_team() {
    assert_eq!(
        team::parse("team = 'proj'\nprivate = false\n[[session]]\nname = 'architect'\ncommand = ['cat']\nallow_agents = ['other/peer']", Path::new("/work/proj"), false)
            .unwrap_err()
            .to_string(),
        "session proj-architect: allow_agents needs a private team"
    );
}
