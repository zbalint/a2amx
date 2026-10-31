//! Team formats and lifecycle through the public module and real daemon.

mod common;

use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::Duration;

use common::pty::PtyHarness;

use a2amx::client::Client;
use a2amx::harness::{Deliver, Harness};
use a2amx::team::{self, TeamSession};
use a2amx::wire::{Request, Response, SessionSummary};

#[test]
fn global_team_name_is_normalized_valid_and_bounded() {
    assert_eq!(
        team::team_name_for(Path::new("/work/My_Project.v2")).unwrap(),
        "my-project-v2"
    );
    assert_eq!(team::team_name_for(Path::new("/work/ABC")).unwrap(), "abc");
    for path in ["/work/s12", "/work/---"] {
        let error = team::team_name_for(Path::new(path))
            .unwrap_err()
            .to_string();
        assert!(error.contains(path));
        assert!(error.contains("--file"));
    }
    let path = format!("/work/{}-{}", "a".repeat(39), "b".repeat(30));
    assert_eq!(
        team::team_name_for(Path::new(&path)).unwrap(),
        "a".repeat(39)
    );
    let bounded = team::team_name_for(Path::new(&path)).unwrap();
    let sessions = team::parse_with(
        "[[session]]\nname = 'architect'\ncommand = ['cat']\n[[session]]\nname = 'consultant'\ncommand = ['cat']",
        Some(&bounded),
    )
    .unwrap();
    assert_eq!(sessions[0].name, format!("{bounded}-architect"));
    assert_eq!(sessions[1].name, format!("{bounded}-consultant"));
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
fn parse_with_applies_derived_private_team_and_rejects_global_team_key() {
    let text = r#"
[[session]]
name = "architect"
command = ["cat"]
[[session]]
name = "consultant"
command = ["cat"]
"#;
    let sessions = team::parse_with(text, Some("proj")).unwrap();
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
    let error = team::parse_with(
        "team = 'wrong'\n[[session]]\nname = 'a'\ncommand = ['cat']",
        Some("proj"),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("must not set team"));
    assert!(
        team::parse(text)
            .unwrap()
            .iter()
            .all(|session| session.team.is_none())
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
            error.contains(": the daemon state directory or a system directory;"),
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
fn parse_global_team_conflict_precedes_prefix() {
    let error = team::parse_with("team = 'demo'\nprefix = 'old'", Some("project"))
        .unwrap_err()
        .to_string();
    assert_eq!(
        error,
        "the global team file must not set team; the team is named after the directory"
    );
}

#[test]
fn parse_reports_invalid_team_name_with_team_prefix() {
    let error = team::parse("team = 'bad name!'").unwrap_err().to_string();
    assert!(error.starts_with("team \"bad name!\": "), "{error}");
}

#[test]
fn parse_reports_invalid_allow_entry_with_entry_prefix() {
    let error = team::parse("team = 'demo'\nallow = ['bad name!']")
        .unwrap_err()
        .to_string();
    assert!(error.starts_with("allow entry \"bad name!\": "), "{error}");
}

#[test]
fn parse_prefix_precedes_private_without_team() {
    let error = team::parse("prefix = 'old'\nprivate = true")
        .unwrap_err()
        .to_string();
    assert_eq!(error, "prefix was renamed to team; see the README");
}

#[test]
fn parse_characterizes_private_requires_team() {
    assert_eq!(
        team::parse("private = true").unwrap_err().to_string(),
        "private requires team"
    );
}

#[test]
fn parse_characterizes_allow_requires_team() {
    assert_eq!(
        team::parse("allow = []").unwrap_err().to_string(),
        "allow requires team"
    );
}

#[test]
fn parse_characterizes_empty_team() {
    assert_eq!(
        team::parse("team = ''").unwrap_err().to_string(),
        "team must not be empty"
    );
}

#[test]
fn parse_characterizes_team_trailing_dash() {
    assert_eq!(
        team::parse("team = 'demo-'").unwrap_err().to_string(),
        "team \"demo-\" must not end in '-'; the dash is added for you"
    );
}

#[test]
fn parse_public_team_precedes_allow_validation() {
    assert_eq!(
        team::parse("team = 'demo'\nprivate = false\nallow = ['demo', 'demo']")
            .unwrap_err()
            .to_string(),
        "allow needs a private team"
    );
}

#[test]
fn parse_characterizes_self_allow() {
    assert_eq!(
        team::parse("team = 'demo'\nallow = ['demo']")
            .unwrap_err()
            .to_string(),
        "allow entry must not be the team itself"
    );
}

#[test]
fn parse_characterizes_repeated_allow() {
    assert_eq!(
        team::parse("team = 'demo'\nallow = ['other', 'other']")
            .unwrap_err()
            .to_string(),
        "allow entry \"other\" is repeated"
    );
}

#[test]
fn parse_characterizes_role_name_error() {
    let error = team::parse("[[session]]\nrole = 'Bad'\ncommand = ['cat']")
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
        team::parse("[[session]]\ncommand = ['cat']")
            .unwrap_err()
            .to_string(),
        "session 1: name or role is required"
    );
}

#[test]
fn parse_characterizes_session_allow_without_team() {
    assert_eq!(
        team::parse("[[session]]\nname = 'architect'\ncommand = ['cat']\nallow = ['other/peer']")
            .unwrap_err()
            .to_string(),
        "session architect: allow needs a private team"
    );
}

fn summary(name: &str, id: &str, exit_code: Option<i32>) -> SessionSummary {
    SessionSummary {
        id: id.into(),
        name: Some(name.into()),
        argv: vec!["cat".into()],
        cols: 80,
        rows: 24,
        attached: false,
        exit_code,
        pending: 0,
        address: String::new(),
        held: false,
        hold_reason: None,
        quota: None,
        harness: Default::default(),
        cwd: None,
        activity: None,
        team: None,
        uptime_secs: None,
        activity_secs: None,
    }
}
#[test]
fn reset_targets_preserve_order_and_protect_attached_sessions() {
    use team::{ResetState, ResetTarget};

    let names = ["a", "b", "c", "d", "e"].map(String::from);
    let mut attached = summary("c", "c-id", None);
    attached.attached = true;
    let existing = [
        attached,
        summary("b", "b-id", Some(0)),
        summary("a", "a-id", None),
        summary("d", "d-id", None),
    ];
    let except = ["d", "unselected"].map(String::from);
    assert_eq!(
        team::reset_targets(&names, false, &except, false, &existing),
        vec![
            ResetTarget {
                name: "a".into(),
                state: ResetState::Reset { id: "a-id".into() },
            },
            ResetTarget {
                name: "b".into(),
                state: ResetState::Exited { id: "b-id".into() },
            },
            ResetTarget {
                name: "c".into(),
                state: ResetState::SkippedAttached { id: "c-id".into() },
            },
            ResetTarget {
                name: "e".into(),
                state: ResetState::Missing,
            },
        ]
    );
    for (explicit, include_attached) in [(false, true), (true, false)] {
        assert_eq!(
            team::reset_targets(&names, explicit, &except, include_attached, &existing),
            vec![
                ResetTarget {
                    name: "a".into(),
                    state: ResetState::Reset { id: "a-id".into() },
                },
                ResetTarget {
                    name: "b".into(),
                    state: ResetState::Exited { id: "b-id".into() },
                },
                ResetTarget {
                    name: "c".into(),
                    state: ResetState::Reset { id: "c-id".into() },
                },
                ResetTarget {
                    name: "e".into(),
                    state: ResetState::Missing,
                },
            ]
        );
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_team_file_is_used_with_local_precedence_and_guard() {
    let (state, _daemon) = common::start_daemon().await;
    let project_root = tempfile::tempdir().unwrap();
    let project = project_root.path().join("proj");
    std::fs::create_dir(&project).unwrap();
    let user_home = tempfile::tempdir().unwrap();
    std::fs::write(
        state.path().join("team.toml"),
        "[[session]]\nname = 'architect'\ncommand = ['cat']",
    )
    .unwrap();
    let global = run_binary_in(state.path(), &project, &["team", "status"]).unwrap();
    assert!(
        global.status.success(),
        "{}",
        String::from_utf8_lossy(&global.stderr)
    );
    assert!(String::from_utf8_lossy(&global.stderr).contains("using global team file"));
    assert!(String::from_utf8_lossy(&global.stdout).contains("proj-architect"));
    std::fs::write(
        project.join("a2amx.toml"),
        "[[session]]\nname = 'architect'\ncommand = ['cat']",
    )
    .unwrap();
    let local = run_binary_in(state.path(), &project, &["team", "status"]).unwrap();
    assert!(local.status.success());
    assert!(String::from_utf8_lossy(&local.stdout).contains("architect"));
    assert!(!String::from_utf8_lossy(&local.stderr).contains("using global team file"));
    let guarded = run_binary_in_with_home(
        state.path(),
        user_home.path(),
        user_home.path(),
        &["team", "status"],
    )
    .unwrap();
    assert!(!guarded.status.success());
    assert!(String::from_utf8_lossy(&guarded.stderr).contains("your home directory"));
}

#[test]
fn global_team_file_missing_error_names_both_paths() {
    let state = tempfile::tempdir().unwrap();
    let project_root = tempfile::tempdir().unwrap();
    let project = project_root.path().join("proj");
    std::fs::create_dir(&project).unwrap();
    let output = run_binary_in(state.path(), &project, &["team", "up"]).unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("a2amx.toml"));
    assert!(error.contains("team.toml"));
    assert!(!error.contains("start it"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_team_relative_cwd_uses_invoking_directory() {
    let (state, _daemon) = common::start_daemon().await;
    let project_root = tempfile::tempdir().unwrap();
    let project = project_root.path().join("proj");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(
        state.path().join("team.toml"),
        "[[session]]\nname = 'developer'\ncommand = ['sh', '-c', 'sleep 30']\ncwd = '.'",
    )
    .unwrap();
    let output = run_binary_in(state.path(), &project, &["team", "up", "--detach"]).unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let listed = run_binary_in(state.path(), &project, &["list", "--details"]).unwrap();
    assert!(String::from_utf8_lossy(&listed.stdout).contains(&project.display().to_string()));
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn local_team_file_symlink_observes_canonical_current_dir() {
    let (state, _daemon) = common::start_daemon().await;
    let root = tempfile::tempdir().unwrap();
    let real = root.path().join("real-project");
    let link = root.path().join("project-link");
    std::fs::create_dir(&real).unwrap();
    std::fs::write(
        real.join("a2amx.toml"),
        "[[session]]\nname = 'developer'\ncommand = ['sh', '-c', 'sleep 30']\ncwd = '.'",
    )
    .unwrap();
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let output = run_binary_in(state.path(), &link, &["team", "up", "--detach"]).unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let listed = run_binary_in(state.path(), &link, &["list", "--details"]).unwrap();
    let details = String::from_utf8_lossy(&listed.stdout);
    // Linux current_dir() resolves a symlink before the CLI receives it.
    assert!(
        details.contains(&real.display().to_string()),
        "expected canonical project path {}, got:\n{details}",
        real.display()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_team_file_bypasses_global_fallback_and_guard() {
    let (state, _daemon) = common::start_daemon().await;
    let user_home = tempfile::tempdir().unwrap();
    std::fs::write(
        state.path().join("team.toml"),
        "[[session]]\nname = 'global'\ncommand = ['cat']",
    )
    .unwrap();
    let explicit = user_home.path().join("explicit.toml");
    std::fs::write(
        &explicit,
        "[[session]]\nname = 'explicit'\ncommand = ['cat']",
    )
    .unwrap();
    let explicit_path = explicit.to_string_lossy().into_owned();
    let output = run_binary_in_with_home(
        state.path(),
        user_home.path(),
        user_home.path(),
        &["team", "status", "--file", &explicit_path],
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("explicit"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("global"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("using global team file"));
    let missing = user_home.path().join("missing.toml");
    let missing_path = missing.to_string_lossy().into_owned();
    let output = run_binary_in_with_home(
        state.path(),
        user_home.path(),
        user_home.path(),
        &["team", "status", "--file", &missing_path],
    )
    .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains(&missing_path));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("using global team file"));
}

fn run_binary_in_with_home(
    home: &Path,
    cwd: &Path,
    user_home: &Path,
    args: &[&str],
) -> std::io::Result<Output> {
    Command::new(env!("CARGO_BIN_EXE_a2amx"))
        .arg("--home")
        .arg(home)
        .args(args)
        .current_dir(cwd)
        .env("HOME", user_home)
        .stdin(Stdio::null())
        .output()
}

fn run_binary(home: &Path, args: &[&str]) -> std::io::Result<Output> {
    Command::new(env!("CARGO_BIN_EXE_a2amx"))
        .arg("--home")
        .arg(home)
        .args(args)
        .current_dir(home)
        .stdin(Stdio::null())
        .output()
}

fn run_binary_in(home: &Path, cwd: &Path, args: &[&str]) -> std::io::Result<Output> {
    Command::new(env!("CARGO_BIN_EXE_a2amx"))
        .arg("--home")
        .arg(home)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .output()
}

const LIVE_TEAM: &str = r#"
[[session]]
name = "architect"
command = ["sh", "-c", "sleep 30"]
[[session]]
name = "developer"
command = ["sh", "-c", "sleep 30"]
"#;

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
    )
    .unwrap();
    assert_eq!(
        actual,
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
                role: Some("architect".into()),
                allow: Vec::new(),
                confine: None,
                team: None,
            },
            TeamSession {
                name: "developer".into(),
                command: vec!["omp".into()],
                cwd: Some(".".into()),
                attach: false,
                reset: None,
                control_from: Vec::new(),
                watch: Vec::new(),
                heartbeat: None,
                activity: None,
                role: None,
                allow: Vec::new(),
                confine: None,
                team: None,
            },
        ]
    );
}

#[test]
fn parse_resolves_agent_allow_only_for_the_declaring_session() {
    let sessions = team::parse(
        r#"team = "a2amx"
allow = ["other"]
[[session]]
name = "architect"
command = ["cat"]
allow = ["saltmdb/architect"]
[[session]]
name = "developer"
command = ["cat"]
"#,
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
            "team = \"a2amx\"\n[[session]]\nname = \"architect\"\ncommand = [\"cat\"]\nallow = [\"{allow}\"]"
        );
        let error = team::parse(&text).expect_err(entry);
        assert!(error.to_string().contains("session"), "{entry}: {error}");
        assert!(error.to_string().contains("allow"), "{entry}: {error}");
    }
    let entries = (0..17)
        .map(|index| format!("\"saltmdb/agent{index}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let error = team::parse(&format!(
        "team = \"a2amx\"\n[[session]]\nname = \"architect\"\ncommand = [\"cat\"]\nallow = [{entries}]"
    ))
    .expect_err("allow cap");
    assert!(error.to_string().contains("session"));
    assert!(error.to_string().contains("allow"));
    let error = team::parse(
        "team = \"a2amx\"\nprivate = false\n[[session]]\nname = \"architect\"\ncommand = [\"cat\"]\nallow = [\"saltmdb/architect\"]",
    )
    .expect_err("public team");
    assert!(error.to_string().contains("session a2amx-architect"));
    assert!(error.to_string().contains("allow"));
    let error = team::parse(
        "[[session]]\nname = \"architect\"\ncommand = [\"cat\"]\nallow = [\"saltmdb/architect\"]",
    )
    .expect_err("ungrouped session");
    assert!(error.to_string().contains("session architect"));
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
    )
    .unwrap();
    assert_eq!(sessions[0].name, "architect");
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
    )
    .unwrap();
    assert_eq!(sessions[0].name, "worker");
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
        let error = team::parse(&text).expect_err(role);
        let error = error.to_string();
        assert!(error.contains(expected), "{error}");
        assert!(error.contains("set name explicitly"), "{error}");
    }
}

#[test]
fn parse_requires_name_or_role() {
    let error = team::parse("[[session]]\ncommand = [\"cat\"]").expect_err("missing identity");
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
        let error = team::parse(text).expect_err(text);
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
            let error = team::parse(&text).expect_err(&text);
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
    let sessions = team::parse(
        "[[session]]\nname = \"architect\"\ncommand = [\"cat\"]\nwatch = [\"outsider\", \"other/architect\", \"/hand\"]",
    ).unwrap();
    assert_eq!(sessions[0].watch, ["outsider", "other-architect", "hand"]);
}

#[test]
fn parse_rejects_invalid_team_keys_and_old_prefix() {
    for (keys, label) in [
        ("prefix = \"x\"", "renamed to team"),
        ("team = \"\"", "team"),
        ("team = \"demo-\"", "team"),
        ("team = 5", "team"),
        ("private = true", "private"),
        ("private = false", "private"),
        ("allow = [\"x\"]", "allow"),
        ("allow = []", "allow"),
        ("team = \"demo\"\nprivate = false\nallow = [\"x\"]", "allow"),
        ("team = \"demo\"\nprivate = false\nallow = []", "allow"),
        ("team = \"demo\"\nallow = [\"demo\"]", "allow"),
        ("team = \"demo\"\nallow = [\"x\", \"x\"]", "allow"),
        ("team = \"demo\"\nallow = [\"Bad\"]", "allow"),
    ] {
        let text = format!("{keys}\n[[session]]\nname = \"architect\"\ncommand = [\"cat\"]");
        let error = team::parse(&text).expect_err(&text);
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
        let error = team::parse(&text).expect_err(&text);
        assert!(format!("{error:#}").contains(final_name), "{error:#}");
    }
}

#[test]
fn parse_rejects_prefixed_self_watch() {
    let error = team::parse(
        "team = \"a2amx\"\n[[session]]\nname = \"developer\"\ncommand = [\"cat\"]\nwatch = [\"developer\"]",
    )
    .expect_err("prefixed self watch");
    assert!(error.to_string().contains("watches itself"), "{error:#}");
}

#[test]
fn parse_rejects_whitespace_in_executable_and_preserves_argument_boundaries() {
    for executable in ["claude --model opus", "/tmp/my executable", "cat\\t-n"] {
        let text = format!(
            "team = \"pfx\"\n[[session]]\nname = \"architect\"\ncommand = [\"{executable}\"]"
        );
        let error = team::parse(&text).expect_err(&text);
        let error = format!("{error:#}");
        assert!(
            error.contains("session pfx-architect: command[0]"),
            "{error}"
        );
        assert!(error.contains("own array element"), "{error}");
    }
    let sessions = team::parse(
        "[[session]]\nname = \"architect\"\ncommand = [\"claude\", \"--model\", \"opus\"]",
    )
    .unwrap();
    assert_eq!(sessions[0].command, ["claude", "--model", "opus"]);
}

#[test]
fn parse_rejects_invalid_sessions_and_identifies_the_entry() {
    for (text, label) in [
        ("[[session]]\ncommand = [\"cat\"]", "session 1"),
        ("[[session]]\nname = \"\"\ncommand = [\"cat\"]", "session 1"),
        (
            "[[session]]\nname = \"Bad_Name\"\ncommand = [\"cat\"]",
            "Bad_Name",
        ),
        (
            "[[session]]\nname = \"a\"\ncommand = [\"cat\"]\n[[session]]\nname = \"a\"\ncommand = [\"cat\"]",
            "a",
        ),
        ("[[session]]\nname = \"a\"\ncommand = []", "a"),
        (
            "[[session]]\nname = \"a\"\ncommand = [\"cat\"]\nattach = true\n[[session]]\nname = \"b\"\ncommand = [\"cat\"]\nattach = true",
            "b",
        ),
        (
            "[[session]]\nname = \"a\"\ncommand = [\"cat\"]\nextra = true",
            "session 1",
        ),
        ("[[session]]\nname = \"a\"\ncommand = \"cat\"", "session 1"),
    ] {
        let error = team::parse(text).expect_err(text);
        assert!(format!("{error:#}").contains(label), "{error:#}");
    }
    assert!(team::parse("").is_err());
    assert!(team::parse("unexpected = true").is_err());
}

#[test]
fn parse_validates_watch_names_and_self_reference() {
    let parsed = team::parse(
        r#"
[[session]]
name = "architect"
command = ["cat"]
watch = ["developer"]
"#,
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
            "watch entry",
        ),
    ] {
        let error = team::parse(text).expect_err(text);
        assert!(format!("{error:#}").contains(label), "{error:#}");
    }
}

#[test]
fn parse_rejects_the_daemon_sender_name() {
    let error = team::parse("[[session]]\nname = \"a2amx-daemon\"\ncommand = [\"cat\"]")
        .expect_err("daemon sender name");
    assert!(error.to_string().contains("reserved for the daemon"));
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
                allow: Vec::new(),
                confine: None,
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
                allow: Vec::new(),
                confine: None,
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
grants = ["containers"]
"#,
    )
    .unwrap();
    let request = sessions[0].confine.as_ref().expect("confined");
    assert_eq!(request.write, ["./data"]);
    assert!(request.grants.contains(&"containers".to_owned()));
    let error = team::parse(
        r#"
[[session]]
name = "worker"
command = ["cat"]
[session.confine]
hide = ["./secrets"]
"#,
    )
    .expect_err("options without enabled");
    assert!(error.to_string().contains("need enabled = true"));
    let error = team::parse(
        r#"
[[session]]
name = "worker"
command = ["codex"]
[session.confine]
enabled = true
"#,
    )
    .expect_err("codex confinement");
    assert!(error.to_string().contains("not supported for Codex"));
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
    )
    .expect("post-separator prompt option is not part of the command");
    assert!(sessions[0].confine.is_some());
}

#[test]
fn plan_preserves_wanted_order_and_preflights_every_exited_conflict() {
    let wanted = team::flag_sessions(&["architect=cat".into(), "developer=cat".into()]).unwrap();
    assert_eq!(
        team::plan(&wanted, &[summary("developer", "s2", None)]).unwrap(),
        [
            team::Action::Start(Box::new(wanted[0].clone())),
            team::Action::AlreadyRunning {
                name: "developer".into(),
                id: "s2".into()
            },
        ],
    );
    assert_eq!(
        team::plan(&wanted, &[summary("developer", "s2", Some(0))]).unwrap_err(),
        [team::Conflict {
            name: "developer".into(),
            id: "s2".into(),
            reason: team::ConflictReason::Exited,
        }],
    );
    assert_eq!(
        team::plan(
            &wanted,
            &[
                summary("developer", "s2", Some(1)),
                summary("architect", "s1", Some(0))
            ]
        )
        .unwrap_err(),
        [
            team::Conflict {
                name: "architect".into(),
                id: "s1".into(),
                reason: team::ConflictReason::Exited,
            },
            team::Conflict {
                name: "developer".into(),
                id: "s2".into(),
                reason: team::ConflictReason::Exited,
            },
        ],
    );
}

#[test]
fn plan_reports_running_team_setting_mismatch() {
    let wanted =
        team::parse("team = \"demo\"\n[[session]]\nname = \"worker\"\ncommand = [\"cat\"]")
            .unwrap();
    let mut existing = summary("demo-worker", "s1", None);
    existing.team = Some(a2amx::messaging::TeamScope {
        name: "demo".into(),
        private: false,
        allow: Vec::new(),
        agents: Vec::new(),
    });
    let conflict = team::plan(&wanted, &[existing]).unwrap_err();
    assert_eq!(
        conflict,
        [team::Conflict {
            name: "demo-worker".into(),
            id: "s1".into(),
            reason: team::ConflictReason::TeamMismatch,
        }]
    );
}
#[test]
fn plan_notices_agent_scope_mismatch_and_accepts_matching_scope() {
    let wanted = team::parse(
        "team = \"a2amx\"\n[[session]]\nname = \"architect\"\ncommand = [\"cat\"]\nallow = [\"saltmdb/architect\"]",
    )
    .unwrap();
    let mut existing = summary("a2amx-architect", "s1", None);
    existing.team = Some(a2amx::messaging::TeamScope {
        name: "a2amx".into(),
        private: true,
        allow: Vec::new(),
        agents: Vec::new(),
    });
    assert_eq!(
        team::plan(&wanted, &[existing.clone()]),
        Err(vec![team::Conflict {
            name: "a2amx-architect".into(),
            id: "s1".into(),
            reason: team::ConflictReason::TeamMismatch,
        }])
    );
    existing.team.as_mut().unwrap().agents = vec!["saltmdb-architect".into()];
    assert_eq!(
        team::plan(&wanted, &[existing]),
        Ok(vec![team::Action::AlreadyRunning {
            name: "a2amx-architect".into(),
            id: "s1".into(),
        }])
    );
}

#[test]
fn inspect_preserves_order_and_plan_reports_the_same_conflicts() {
    let wanted = team::flag_sessions(&[
        "a=cat".into(),
        "b=cat".into(),
        "c=cat".into(),
        "d=cat".into(),
    ])
    .unwrap();
    let mut mismatch = summary("c", "c-id", None);
    mismatch.team = Some(a2amx::messaging::TeamScope {
        name: "different".into(),
        private: true,
        allow: Vec::new(),
        agents: Vec::new(),
    });
    let existing = [
        mismatch,
        summary("b", "b-id", Some(0)),
        summary("a", "a-id", None),
    ];
    assert_eq!(
        team::inspect(&wanted, &existing),
        [
            team::Entry {
                name: "a".into(),
                state: team::EntryState::Running { id: "a-id".into() },
            },
            team::Entry {
                name: "b".into(),
                state: team::EntryState::Conflict {
                    id: "b-id".into(),
                    reason: team::ConflictReason::Exited,
                },
            },
            team::Entry {
                name: "c".into(),
                state: team::EntryState::Conflict {
                    id: "c-id".into(),
                    reason: team::ConflictReason::TeamMismatch,
                },
            },
            team::Entry {
                name: "d".into(),
                state: team::EntryState::Missing,
            },
        ]
    );
    assert_eq!(
        team::plan(&wanted, &existing).unwrap_err(),
        [
            team::Conflict {
                name: "b".into(),
                id: "b-id".into(),
                reason: team::ConflictReason::Exited,
            },
            team::Conflict {
                name: "c".into(),
                id: "c-id".into(),
                reason: team::ConflictReason::TeamMismatch,
            },
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_reports_running_and_missing_without_changing_sessions() {
    let (dir, _daemon) = common::start_daemon().await;
    std::fs::write(
        dir.path().join("up.toml"),
        r#"
[[session]]
name = "running"
command = ["sh", "-c", "sleep 30"]
"#,
    )
    .unwrap();
    let up = run_binary(dir.path(), &["team", "up", "--detach", "--file", "up.toml"]).unwrap();
    assert!(
        up.status.success(),
        "{}",
        String::from_utf8_lossy(&up.stderr)
    );
    std::fs::write(
        dir.path().join("team.toml"),
        r#"
[[session]]
name = "running"
command = ["sh", "-c", "sleep 30"]
[[session]]
name = "missing"
command = ["cat"]
"#,
    )
    .unwrap();
    let before = run_binary(dir.path(), &["list"]).unwrap().stdout;

    let status = run_binary(dir.path(), &["team", "status", "--file", "team.toml"]).unwrap();
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    assert!(status.stderr.is_empty());
    let stdout = String::from_utf8(status.stdout).unwrap();
    assert!(stdout.contains("NAME TEAM STATE NOTE\n"));
    assert!(stdout.contains("running - running s1\n"));
    assert!(stdout.contains("missing - missing -\n"));

    let after = run_binary(dir.path(), &["list"]).unwrap().stdout;
    assert_eq!(after, before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_reports_conflicts_without_a_generic_error_line() {
    let (dir, _daemon) = common::start_daemon().await;
    assert!(
        run_binary(
            dir.path(),
            &[
                "new", "--detach", "--name", "exited", "--", "sh", "-c", "exit 0"
            ],
        )
        .unwrap()
        .status
        .success()
    );
    common::eventually(|| async {
        let output = run_binary(dir.path(), &["list"]).unwrap();
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("exited(0)")
            .then_some(())
    })
    .await;
    std::fs::write(
        dir.path().join("exited.toml"),
        "[[session]]\nname = 'exited'\ncommand = ['cat']",
    )
    .unwrap();
    let exited = run_binary(dir.path(), &["team", "status", "--file", "exited.toml"]).unwrap();
    assert_eq!(exited.status.code(), Some(1));
    let exited_stdout = String::from_utf8(exited.stdout).unwrap();
    assert!(exited_stdout.contains("exited - exited"));
    assert!(exited_stdout.contains("run a2amx kill exited first"));
    assert!(!String::from_utf8_lossy(&exited.stderr).starts_with("a2amx:"));

    let mismatch_team = r#"team = "x"
[[session]]
name = "same"
command = ["sleep", "30"]
"#;
    std::fs::write(dir.path().join("mismatch-up.toml"), mismatch_team).unwrap();
    let up = run_binary(
        dir.path(),
        &["team", "up", "--detach", "--file", "mismatch-up.toml"],
    )
    .unwrap();
    assert!(
        up.status.success(),
        "{}",
        String::from_utf8_lossy(&up.stderr)
    );
    std::fs::write(
        dir.path().join("mismatch-status.toml"),
        "team = 'x'\nprivate = false\n[[session]]\nname = 'same'\ncommand = ['cat']",
    )
    .unwrap();
    let mismatch = run_binary(
        dir.path(),
        &["team", "status", "--file", "mismatch-status.toml"],
    )
    .unwrap();
    assert_eq!(mismatch.status.code(), Some(1));
    let mismatch_stdout = String::from_utf8(mismatch.stdout).unwrap();
    assert!(mismatch_stdout.contains("same x team-mismatch"));
    assert!(mismatch_stdout.contains("run a2amx team down first"));
    assert!(!String::from_utf8_lossy(&mismatch.stderr).starts_with("a2amx:"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dry_run_prints_wired_commands_without_starting_or_installing() {
    let (dir, _daemon) = common::start_daemon().await;
    std::fs::write(
        dir.path().join("dry.toml"),
        r#"
[[session]]
name = "claude"
command = ["claude", "--settings", "{\"x\":\"UNIQUE-SECRET\"}"]
cwd = "."
role = "developer"
attach = true
watch = ["other"]
control_from = ["other"]
heartbeat = "5s"
activity = { working_after = "2m", idle_after = "30s" }
[[session]]
name = "generic"
command = ["sh", "-c", "echo generic"]
[[session]]
name = "omp"
command = ["omp", "--secret"]
"#,
    )
    .unwrap();
    let output = run_binary(
        dir.path(),
        &["team", "up", "--dry-run", "--file", "dry.toml"],
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("would start claude\n"));
    assert!(stdout.contains("would start generic\n"));
    assert!(stdout.contains("would start omp\n"));
    assert!(stdout.contains("  harness: claude\n"));
    assert!(stdout.contains("  cwd: "));
    assert!(stdout.contains("  role: developer\n"));
    assert!(stdout.contains("  attach: true\n"));
    assert!(stdout.contains("  watch: other\n"));
    assert!(stdout.contains("  control_from: other\n"));
    assert!(stdout.contains("  heartbeat: 5s\n"));
    assert!(stdout.contains("  activity: working_after 2m, idle_after 30s\n"));
    assert!(stdout.contains("  command: claude "));
    assert!(stdout.contains("--mcp-config "));
    assert!(stdout.contains("--settings "));
    assert!(stdout.contains("--append-system-prompt "));
    assert!(stdout.contains("\nomp extension install skipped (dry run)\n"));
    assert!(!stdout.contains("UNIQUE-SECRET"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("UNIQUE-SECRET"));
    assert!(!dir.path().join("omp").exists());
    let listed = run_binary(dir.path(), &["list"]).unwrap();
    assert!(listed.status.success());
    assert!(!String::from_utf8_lossy(&listed.stdout).contains("claude"));
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dry_run_expands_home_relative_confine_paths() {
    let (state, _daemon) = common::start_daemon().await;
    let project = tempfile::tempdir().unwrap();
    let user_home = tempfile::tempdir().unwrap();
    let runtime_dir = tempfile::tempdir().unwrap();
    let tool_cache = user_home.path().join(".tool-cache");
    let tool_hide = user_home.path().join(".tool-hide");
    let ssh = user_home.path().join(".ssh");
    std::fs::create_dir(&tool_cache).unwrap();
    std::fs::create_dir(&tool_hide).unwrap();
    std::fs::create_dir(&ssh).unwrap();
    std::fs::write(
        project.path().join("dry-tilde.toml"),
        r#"
        [confine]
        enabled = true
        [[session]]
        name = "demo"
        command = ["sh", "-c", "echo demo"]
        cwd = "."
        [session.confine]
        write = ["~/.tool-cache", "~/.missing-tool"]
        hide = ["~/.tool-hide"]
        unhide = ["~/.ssh"]
        grants = ["containers"]
        "#,
    )
    .unwrap();
    let output = run_binary_in_with_home_and_runtime(
        state.path(),
        project.path(),
        user_home.path(),
        runtime_dir.path(),
        &["team", "up", "--dry-run", "--file", "dry-tilde.toml"],
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let writable = std::fs::canonicalize(&tool_cache).unwrap();
    let hidden = std::fs::canonicalize(&tool_hide).unwrap();
    let unhidden = std::fs::canonicalize(&ssh).unwrap();
    let missing = user_home.path().join(".missing-tool");
    assert!(stdout.contains("  grants: containers\n"));
    assert!(!stdout.contains(&format!("  hidden: {}\n", unhidden.display())));
    assert!(stdout.contains(&format!("  writable: {}\n", writable.display())));
    assert!(stdout.contains(&format!("  hidden: {}\n", hidden.display())));
    assert!(!stdout.contains("/~/"));
    assert!(!stdout.contains("~/"));
    assert!(!stdout.contains(&format!("  writable: {}\n", missing.display())));
    assert!(stdout.contains(&format!(
        "  unresolved: {} (write: not found)\n",
        missing.display()
    )));
    assert!(!stdout.contains(&format!(
        "  hidden: {} (skipped missing)\n",
        missing.display()
    )));
    let default_missing = user_home.path().join(".aws");
    assert!(stdout.contains(&format!(
        "  hidden: {} (skipped missing)\n",
        default_missing.display()
    )));
    std::fs::write(
        project.path().join("dry-plain.toml"),
        r#"
        [confine]
        enabled = true
        [[session]]
        name = "plain"
        command = ["sh", "-c", "echo plain"]
        cwd = "."
        [session.confine]
        write = ["~/.tool-cache"]
        "#,
    )
    .unwrap();
    let plain_output = run_binary_in_with_home_and_runtime(
        state.path(),
        project.path(),
        user_home.path(),
        runtime_dir.path(),
        &["team", "up", "--dry-run", "--file", "dry-plain.toml"],
    )
    .unwrap();
    assert!(
        plain_output.status.success(),
        "{}",
        String::from_utf8_lossy(&plain_output.stderr)
    );
    let plain_stdout = String::from_utf8(plain_output.stdout).unwrap();
    assert!(plain_stdout.contains("  confinement: confined\n"));
    assert!(!plain_stdout.contains("  grants: "));
}

fn run_binary_in_with_home_and_runtime(
    home: &Path,
    cwd: &Path,
    user_home: &Path,
    runtime_dir: &Path,
    args: &[&str],
) -> std::io::Result<Output> {
    Command::new(env!("CARGO_BIN_EXE_a2amx"))
        .arg("--home")
        .arg(home)
        .args(args)
        .current_dir(cwd)
        .env("HOME", user_home)
        .env("XDG_RUNTIME_DIR", runtime_dir)
        .stdin(Stdio::null())
        .output()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dry_run_rejects_existing_relative_omp_prompt_file() {
    let (dir, _daemon) = common::start_daemon().await;
    std::fs::write(dir.path().join("prompt.txt"), "prompt").unwrap();
    std::fs::write(
        dir.path().join("invalid-omp.toml"),
        r#"
        [confine]
        enabled = true
        [[session]]
        name = "omp"
        command = ["omp", "--append-system-prompt=prompt.txt"]
        cwd = "."
        "#,
    )
    .unwrap();
    let output = run_binary(
        dir.path(),
        &["team", "up", "--dry-run", "--file", "invalid-omp.toml"],
    )
    .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("existing file as OMP's append-system-prompt value"));
    assert!(!stderr.contains("would start omp"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dry_run_prints_exact_private_team_report() {
    let (dir, _daemon) = common::start_daemon().await;
    std::fs::write(
        dir.path().join("dry.toml"),
        "team = \"t\"\n[[session]]\nname = \"worker\"\ncommand = [\"cat\"]\ncwd = \"/\"\n",
    )
    .unwrap();
    let output = run_binary(
        dir.path(),
        &["team", "up", "--dry-run", "--file", "dry.toml"],
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "would start t-worker\n  harness: generic\n  cwd: /\n  team: t, private, allow: -\n  confinement: unconfined\n  command: cat\n"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dry_run_prints_exact_already_running_report() {
    let (dir, _daemon) = common::start_daemon().await;
    std::fs::write(
        dir.path().join("running.toml"),
        "[[session]]\nname = \"running\"\ncommand = [\"cat\"]\n",
    )
    .unwrap();
    let started = run_binary(
        dir.path(),
        &["team", "up", "--detach", "--file", "running.toml"],
    )
    .unwrap();
    assert!(
        started.status.success(),
        "{}",
        String::from_utf8_lossy(&started.stderr)
    );
    let output = run_binary(
        dir.path(),
        &["team", "up", "--dry-run", "--file", "running.toml"],
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "already running running s1\n"
    );
}

#[test]
fn dry_run_rejects_detach_and_inline_items() {
    let dir = tempfile::tempdir().unwrap();
    for args in [
        vec!["team", "up", "--dry-run", "--detach"],
        vec!["team", "up", "--dry-run", "worker=cat"],
    ] {
        let output = run_binary(dir.path(), &args).unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("cannot be used with"));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dry_run_redacts_flag_values_and_non_flags() {
    let (dir, _daemon) = common::start_daemon().await;
    std::fs::write(
        dir.path().join("redact.toml"),
        r#"
[[session]]
name = "values"
command = ["claude", "--model", "sonnet", "--x=ab", "pos"]
[[session]]
name = "separator"
command = ["claude", "--", "-secret"]
[[session]]
name = "short"
command = ["claude", "-token123"]
"#,
    )
    .unwrap();
    let output = run_binary(
        dir.path(),
        &["team", "up", "--dry-run", "--file", "redact.toml"],
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("command: claude --model <6 bytes> --x=<redacted, 2 bytes> <3 bytes>"));
    assert!(stdout.contains(" -- <7 bytes>"));
    assert!(stdout.contains("command: claude <9 bytes>"));
    assert!(!stdout.contains("sonnet"));
    assert!(!stdout.contains("-secret"));
    assert!(!stdout.contains("-token123"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_notes_messages_lost_in_a_daemon_restart_only_for_wanted_names() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon_in(dir.path(), Some("host-a"), Default::default()).await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_recipient_id, _recipient_token, _recipient_addr) = common::new_agent(
        &mut admin,
        dir.path(),
        Some("wanted"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let (_sender_id, sender_token, sender_addr) = common::new_agent(
        &mut admin,
        dir.path(),
        Some("sender"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let mut sender = Client::connect_addr(sender_addr, &sender_token)
        .await
        .unwrap();
    let accepted = sender
        .request(Request::SendMessage {
            to: "wanted@host-a".into(),
            subject: "restart".into(),
            message: "body".into(),
        })
        .await
        .unwrap();
    assert!(matches!(accepted, Response::Accepted { .. }));
    drop(daemon);

    let restarted = common::start_daemon_in(dir.path(), Some("host-a"), Default::default()).await;
    std::fs::write(
        dir.path().join("wanted.toml"),
        "[[session]]\nname = 'wanted'\ncommand = ['cat']",
    )
    .unwrap();
    let wanted = run_binary(dir.path(), &["team", "status", "--file", "wanted.toml"]).unwrap();
    assert!(wanted.status.success());
    assert!(String::from_utf8_lossy(&wanted.stderr).contains("1 message(s)"));

    std::fs::write(
        dir.path().join("other.toml"),
        "[[session]]\nname = 'other'\ncommand = ['cat']",
    )
    .unwrap();
    let other = run_binary(dir.path(), &["team", "status", "--file", "other.toml"]).unwrap();
    assert!(other.status.success());
    assert!(!String::from_utf8_lossy(&other.stderr).contains("lost in a daemon restart"));

    restarted.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn up_starts_in_order_and_skips_running_sessions_without_attachment() {
    let (dir, _daemon) = common::start_daemon().await;
    std::fs::write(dir.path().join("team.toml"), LIVE_TEAM).unwrap();
    let first = run_binary(
        dir.path(),
        &["team", "up", "--detach", "--file", "team.toml"],
    )
    .unwrap();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(
        first.stdout,
        b"started architect s1\nstarted developer s2\n"
    );
    let listed = run_binary(dir.path(), &["list"]).unwrap();
    let listed = String::from_utf8(listed.stdout).unwrap();
    for (id, name) in [("s1", "architect"), ("s2", "developer")] {
        let cells = common::list_cells(&listed, id);
        assert_eq!(&cells[..4], [id, name, "generic", "running"]);
        assert_eq!(cells[5], "working");
        assert_eq!(cells[7], "no");
        for index in [4, 6] {
            let bytes = cells[index].as_bytes();
            assert!(bytes.len() >= 2);
            assert!(
                bytes[..bytes.len() - 1]
                    .iter()
                    .all(|byte| byte.is_ascii_digit())
            );
            assert!(matches!(bytes.last(), Some(b's' | b'm' | b'h')));
        }
    }
    let second = run_binary(dir.path(), &["team", "up", "--file", "team.toml"]).unwrap();
    assert!(second.status.success());
    assert_eq!(
        second.stdout,
        b"already running architect s1\nalready running developer s2\n"
    );
    let listed_again = run_binary(dir.path(), &["list"]).unwrap();
    assert_eq!(listed_again.stdout, listed.as_bytes());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn down_kills_only_named_team_sessions_and_reports_missing_names() {
    let (dir, _daemon) = common::start_daemon().await;
    std::fs::write(dir.path().join("team.toml"), LIVE_TEAM).unwrap();
    assert!(
        run_binary(
            dir.path(),
            &["team", "up", "--detach", "--file", "team.toml"]
        )
        .unwrap()
        .status
        .success()
    );
    assert!(
        run_binary(
            dir.path(),
            &["new", "--detach", "--name", "unrelated", "--", "cat"]
        )
        .unwrap()
        .status
        .success()
    );
    let down = run_binary(dir.path(), &["team", "down", "--file", "team.toml"]).unwrap();
    assert!(
        down.status.success(),
        "{}",
        String::from_utf8_lossy(&down.stderr)
    );
    assert_eq!(down.stdout, b"killed architect s1\nkilled developer s2\n");
    let repeated = run_binary(dir.path(), &["team", "down", "--file", "team.toml"]).unwrap();
    assert!(repeated.status.success());
    assert_eq!(
        repeated.stdout,
        b"no session architect\nno session developer\n"
    );
    let listed = String::from_utf8(run_binary(dir.path(), &["list"]).unwrap().stdout).unwrap();
    assert!(listed.contains("s3  unrelated  generic  running"));
    assert!(!listed.contains("architect") && !listed.contains("developer"));
    let named = run_binary(dir.path(), &["team", "down", "unrelated", "missing"]).unwrap();
    assert!(named.status.success());
    assert_eq!(named.stdout, b"killed unrelated s3\nno session missing\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prefixed_team_down_uses_full_names_and_hints_only_for_explicit_missing_names() {
    let (dir, _daemon) = common::start_daemon().await;
    std::fs::write(
        dir.path().join("team.toml"),
        format!("team = \"pfx\"\n{LIVE_TEAM}"),
    )
    .unwrap();
    let up = run_binary(
        dir.path(),
        &["team", "up", "--detach", "--file", "team.toml"],
    )
    .unwrap();
    assert!(
        up.status.success(),
        "{}",
        String::from_utf8_lossy(&up.stderr)
    );
    assert_eq!(
        up.stdout,
        b"started pfx-architect s1\nstarted pfx-developer s2\n"
    );
    let named = run_binary(dir.path(), &["team", "down", "architect", "developer"]).unwrap();
    assert!(named.status.success());
    assert_eq!(
        named.stdout,
        b"no session architect\nno session developer\n"
    );
    assert_eq!(
        named.stderr,
        b"hint: names given to team down are full session names (with any team prefix); see a2amx list\n"
    );
    let down = run_binary(dir.path(), &["team", "down", "--file", "team.toml"]).unwrap();
    assert!(
        down.status.success(),
        "{}",
        String::from_utf8_lossy(&down.stderr)
    );
    assert_eq!(
        down.stdout,
        b"killed pfx-architect s1\nkilled pfx-developer s2\n"
    );
    assert!(down.stderr.is_empty());
    let repeated = run_binary(dir.path(), &["team", "down", "--file", "team.toml"]).unwrap();
    assert!(repeated.status.success());
    assert_eq!(
        repeated.stdout,
        b"no session pfx-architect\nno session pfx-developer\n"
    );
    assert!(repeated.stderr.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exited_conflicts_prevent_every_spawn_and_down_removes_exited_sessions() {
    let (dir, _daemon) = common::start_daemon().await;
    assert!(
        run_binary(
            dir.path(),
            &[
                "new", "--detach", "--name", "exited", "--", "sh", "-c", "exit 0"
            ]
        )
        .unwrap()
        .status
        .success()
    );
    common::eventually(|| async {
        let output = run_binary(dir.path(), &["list"]).unwrap();
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("exited(0)")
            .then_some(())
    })
    .await;
    std::fs::write(
        dir.path().join("a2amx.toml"),
        r#"
[[session]]
name = "missing"
command = ["cat"]
[[session]]
name = "exited"
command = ["cat"]
"#,
    )
    .unwrap();
    let up = run_binary(dir.path(), &["team", "up", "--detach"]).unwrap();
    assert!(!up.status.success());
    assert!(up.stdout.is_empty());
    assert!(
        String::from_utf8(up.stderr)
            .unwrap()
            .contains("session exited has exited (s1); run a2amx kill exited first")
    );
    let listed = String::from_utf8(run_binary(dir.path(), &["list"]).unwrap().stdout).unwrap();
    assert!(listed.contains("s1  exited"));
    assert!(!listed.contains("missing") && !listed.contains("s2"));
    let down = run_binary(dir.path(), &["team", "down"]).unwrap();
    assert!(down.status.success());
    assert_eq!(down.stdout, b"no session missing\nkilled exited s1\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn first_spawn_failure_keeps_started_sessions_and_stops_the_team() {
    let (dir, _daemon) = common::start_daemon().await;
    std::fs::write(
        dir.path().join("a2amx.toml"),
        r#"
[[session]]
name = "started"
command = ["cat"]
[[session]]
name = "broken"
command = ["/nonexistent-a2amx-test-executable"]
[[session]]
name = "later"
command = ["cat"]
"#,
    )
    .unwrap();
    let output = run_binary(dir.path(), &["team", "up", "--detach"]).unwrap();
    assert!(!output.status.success());
    assert_eq!(output.stdout, b"started started s1\n");
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("failed broken:")
    );
    let listed = String::from_utf8(run_binary(dir.path(), &["list"]).unwrap().stdout).unwrap();
    assert!(listed.contains("s1  started  generic  running"));
    assert!(!listed.contains("broken") && !listed.contains("later"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn flag_form_starts_bare_executables() {
    let (dir, _daemon) = common::start_daemon().await;
    let output = run_binary(
        dir.path(),
        &["team", "up", "--detach", "architect=cat", "developer=cat"],
    )
    .unwrap();
    assert!(output.status.success());
    assert_eq!(
        output.stdout,
        b"started architect s1\nstarted developer s2\n"
    );
    let listed = String::from_utf8(run_binary(dir.path(), &["list"]).unwrap().stdout).unwrap();
    assert!(listed.contains("s1  architect  generic  running"));
    assert!(listed.contains("s2  developer  generic  running"));
}

#[test]
fn file_and_argument_errors_precede_daemon_connection() {
    let dir = tempfile::tempdir().unwrap();
    let missing = run_binary(dir.path(), &["team", "up"]).unwrap();
    assert!(!missing.status.success());
    let error = String::from_utf8(missing.stderr).unwrap();
    assert!(error.contains("a2amx.toml"));
    assert!(!error.contains("start it"));
    for args in [
        vec!["team", "up", "--file", "absent.toml", "a=cat"],
        vec!["team", "down", "--file", "absent.toml", "a"],
    ] {
        let output = run_binary(dir.path(), &args).unwrap();
        assert!(!output.status.success());
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains("cannot be used with")
        );
    }
    std::fs::write(
        dir.path().join("a2amx.toml"),
        "[[session]]\nname = 'Bad_Name'\ncommand = ['cat']",
    )
    .unwrap();
    let invalid = run_binary(dir.path(), &["team", "up"]).unwrap();
    assert!(!invalid.status.success());
    assert!(
        String::from_utf8(invalid.stderr)
            .unwrap()
            .contains("Bad_Name")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn file_cwd_is_relative_to_its_directory_not_the_invoking_directory() {
    let (dir, _daemon) = common::start_daemon().await;
    let sub = dir.path().join("config").join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(
        dir.path().join("config/team.toml"),
        r#"
[[session]]
name = "developer"
command = ["sh", "-c", "sleep 30"]
cwd = "sub"
"#,
    )
    .unwrap();
    let output = run_binary(
        dir.path(),
        &["team", "up", "--detach", "--file", "config/team.toml"],
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let listed = String::from_utf8(
        run_binary(dir.path(), &["list", "--details"])
            .unwrap()
            .stdout,
    )
    .unwrap();
    assert!(listed.contains(&format!("{}  sh -c sleep 30", sub.display())));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn terminal_attach_honors_file_selection_flag_first_and_detach_override() -> anyhow::Result<()>
{
    let (dir, _daemon) = common::start_daemon().await;
    std::fs::write(
        dir.path().join("team.toml"),
        r#"
[[session]]
name = "architect"
command = ["sh", "-c", "printf architect-ready; cat"]
[[session]]
name = "developer"
command = ["sh", "-c", "printf developer-ready; cat"]
attach = true
"#,
    )?;
    let args = |command: &[&str]| {
        let mut args = vec![
            "--home".to_owned(),
            dir.path().to_string_lossy().into_owned(),
        ];
        args.extend(command.iter().map(|arg| (*arg).to_owned()));
        args
    };
    let wait = Duration::from_secs(10);
    let team_file = dir.path().join("team.toml").to_string_lossy().into_owned();
    let mut selected = PtyHarness::spawn(&args(&["team", "up", "--file", &team_file]), dir.path())?;
    selected.wait_for_text("developer-ready", wait)?;
    assert!(!selected.screen_text().contains("architect-ready"));
    selected.send(&[2, b'd'])?;
    selected.wait_for_text("[detached from s2]", wait)?;
    assert_eq!(selected.wait_exit(wait)?, 0);

    let mut first = PtyHarness::spawn(
        &args(&["team", "up", "architect=cat", "developer=cat"]),
        dir.path(),
    )?;
    first.wait_for_text("architect-ready", wait)?;
    assert!(!first.screen_text().contains("developer-ready"));
    first.send(&[2, b'd'])?;
    first.wait_for_text("[detached from s1]", wait)?;
    assert_eq!(first.wait_exit(wait)?, 0);

    let mut detached = PtyHarness::spawn(
        &args(&["team", "up", "--detach", "--file", &team_file]),
        dir.path(),
    )?;
    assert_eq!(detached.wait_exit(wait)?, 0);
    assert!(
        detached
            .screen_text()
            .contains("already running architect s1")
    );
    assert!(
        detached
            .screen_text()
            .contains("already running developer s2")
    );
    Ok(())
}

#[test]
fn configured_reset_and_controller_are_accepted_by_team_parser() {
    let text = r#"
[[session]]
name = "worker"
command = ["cat"]
reset = ["/clear", "/prewalk restart"]
control_from = ["architect"]
"#;
    let sessions = team::parse(text).expect("valid reset configuration");
    assert_eq!(
        sessions[0].reset,
        Some(vec!["/clear".into(), "/prewalk restart".into()])
    );
    assert_eq!(sessions[0].control_from, ["architect"]);
    assert!(team::parse(&format!("{text}\nunknown = true")).is_err());
    let empty_reset =
        team::parse("[[session]]\nname = \"worker\"\ncommand = [\"cat\"]\nreset = []")
            .expect_err("empty reset must be rejected");
    assert!(empty_reset.to_string().contains("reset"));
    for reset in [
        "reset = [\"clear\"]",
        "reset = [\"/bad\\ncommand\"]",
        &format!("reset = [\"/{}\"]", "x".repeat(200)),
    ] {
        let invalid = format!("[[session]]\nname = \"worker\"\ncommand = [\"cat\"]\n{reset}");
        assert!(team::parse(&invalid).is_err(), "{reset}");
    }
    let invalid_control =
        "[[session]]\nname = \"worker\"\ncommand = [\"cat\"]\ncontrol_from = [\"Bad_Name\"]";
    assert!(team::parse(invalid_control).is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn team_up_forwards_reset_configuration_and_consent() {
    let (dir, _daemon) = common::start_daemon().await;
    let controller_file = dir.path().join("controller.creds");
    let other_file = dir.path().join("other.creds");
    let composer = dir.path().join("team-composer.py");
    std::fs::write(
        &composer,
        r#"import sys
sys.stdout.write("\x1b[?2004h")
sys.stdout.flush()
for raw in sys.stdin.buffer:
    line = raw.decode(errors="replace").replace("\x1b[200~", "").replace("\x1b[201~", "").rstrip("\r\n")
    if line == "/clear":
        print("marker:/clear", flush=True)
"#,
    )
    .unwrap();
    let config = format!(
        r#"
[[session]]
name = "controller"
command = ["sh", "-c", "printf '%s\\n%s\\n' \"$A2AMX_TOKEN\" \"$A2AMX_ADDR\" > '{controller}'; sleep 30"]

[[session]]
name = "other"
command = ["sh", "-c", "printf '%s\\n%s\\n' \"$A2AMX_TOKEN\" \"$A2AMX_ADDR\" > '{other}'; sleep 30"]

[[session]]
name = "target"
command = ["python3", "-u", "{composer}"]
reset = ["/clear"]
control_from = ["controller"]
"#,
        controller = controller_file.display(),
        other = other_file.display(),
        composer = composer.display(),
    );
    std::fs::write(dir.path().join("a2amx.toml"), config).unwrap();
    let up = run_binary(dir.path(), &["team", "up", "--detach"]).unwrap();
    assert!(
        up.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&up.stderr)
    );

    let read_credentials = |path: std::path::PathBuf| async move {
        common::eventually(|| async {
            let text = std::fs::read_to_string(&path).ok()?;
            let mut lines = text.lines();
            let token = lines.next()?.to_owned();
            let address = lines.next()?.parse::<std::net::SocketAddr>().ok()?;
            (token.len() == 64).then_some((token, address))
        })
        .await
    };
    let (controller_token, controller_addr) = read_credentials(controller_file).await;
    let (other_token, other_addr) = read_credentials(other_file).await;
    let mut controller = Client::connect_addr(controller_addr, &controller_token)
        .await
        .unwrap();
    let reset = controller
        .request(Request::Reset {
            session: "target".into(),
        })
        .await
        .unwrap();
    assert!(matches!(reset, Response::Reset { steps: 1 }), "{reset:?}");

    let mut other = Client::connect_addr(other_addr, &other_token)
        .await
        .unwrap();
    let denied = other
        .request(Request::Reset {
            session: "target".into(),
        })
        .await
        .unwrap();
    assert!(matches!(
        denied,
        Response::Failed { code, .. } if code == "not_permitted"
    ));
}

fn assert_validation_error(label: &str, text: &str, expected: &str) {
    let error = match team::parse(text) {
        Ok(_) => panic!("expected {label} error containing {expected:?}"),
        Err(error) => error.to_string(),
    };
    assert!(error.contains(expected), "{label}: {expected}: {error}");
}

#[test]
fn parse_characterizes_invalid_session_name() {
    assert_validation_error(
        "invalid session name",
        "[[session]]\nname = 'Bad'\ncommand = ['cat']",
        "session 1 (Bad): invalid name \"Bad\"",
    );
}

#[test]
fn parse_characterizes_empty_command() {
    assert_validation_error(
        "empty command",
        "[[session]]\nname = 'worker'\ncommand = []",
        "session worker: command must not be empty",
    );
}

#[test]
fn parse_characterizes_command_whitespace() {
    assert_validation_error(
        "command whitespace",
        "[[session]]\nname = 'worker'\ncommand = ['bad cmd']",
        "contains whitespace; give each argument as its own array element",
    );
}

#[test]
fn parse_characterizes_second_attach() {
    assert_validation_error(
        "second attach",
        "[[session]]\nname = 'first'\ncommand = ['cat']\nattach = true\n[[session]]\nname = 'second'\ncommand = ['cat']\nattach = true",
        "session second: at most one session can attach",
    );
}

#[test]
fn parse_characterizes_empty_reset() {
    assert_validation_error(
        "empty reset",
        "[[session]]\nname = 'worker'\ncommand = ['cat']\nreset = []",
        "session worker: reset must contain at least one step",
    );
}

#[test]
fn parse_characterizes_invalid_reset_step() {
    assert_validation_error(
        "invalid reset step",
        "[[session]]\nname = 'worker'\ncommand = ['cat']\nreset = ['not-a-control-key']",
        "session worker: reset step 1 must start with '/'",
    );
}

#[test]
fn parse_characterizes_invalid_control_from() {
    assert_validation_error(
        "invalid control_from",
        "[[session]]\nname = 'worker'\ncommand = ['cat']\ncontrol_from = ['Bad']",
        "session worker control_from entry \"Bad\": invalid name",
    );
}

#[test]
fn parse_characterizes_invalid_watch() {
    assert_validation_error(
        "invalid watch",
        "[[session]]\nname = 'worker'\ncommand = ['cat']\nwatch = ['Bad']",
        "session worker watch entry \"Bad\": invalid name",
    );
}

#[test]
fn parse_characterizes_invalid_heartbeat() {
    assert_validation_error(
        "invalid heartbeat interval",
        "[[session]]\nname = 'worker'\ncommand = ['cat']\nwatch = ['other']\nheartbeat = '0s'",
        "session worker: invalid heartbeat",
    );
}

#[test]
fn parse_characterizes_invalid_role() {
    assert_validation_error(
        "invalid role",
        "[[session]]\nname = 'worker'\ncommand = ['cat']\nrole = ' '",
        "session worker: role must not have leading or trailing whitespace",
    );
}

#[test]
fn parse_characterizes_check_precedence() {
    let cases = [
        (
            "duplicate precedes command",
            "[[session]]\nname = 'same'\ncommand = ['cat']\n[[session]]\nname = 'same'\ncommand = []",
            "session same: duplicate name",
        ),
        (
            "command precedes attach",
            "[[session]]\nname = 'first'\ncommand = ['cat']\nattach = true\n[[session]]\nname = 'second'\ncommand = []\nattach = true",
            "session second: command must not be empty",
        ),
        (
            "attach precedes reset",
            "[[session]]\nname = 'first'\ncommand = ['cat']\nattach = true\n[[session]]\nname = 'second'\ncommand = ['cat']\nattach = true\nreset = []",
            "session second: at most one session can attach",
        ),
        (
            "reset precedes links",
            "[[session]]\nname = 'worker'\ncommand = ['cat']\nreset = []\ncontrol_from = ['Bad']",
            "session worker: reset must contain at least one step",
        ),
        (
            "control_from precedes watch",
            "[[session]]\nname = 'worker'\ncommand = ['cat']\ncontrol_from = ['Bad']\nwatch = ['Bad']\nheartbeat = '0s'",
            "session worker control_from entry \"Bad\": invalid name",
        ),
        (
            "heartbeat precedes activity",
            "[[session]]\nname = 'worker'\ncommand = ['cat']\nwatch = ['other']\nheartbeat = '0s'\nactivity = {}",
            "session worker: invalid heartbeat",
        ),
        (
            "activity precedes role",
            "[[session]]\nname = 'worker'\ncommand = ['cat']\nwatch = ['other']\nactivity = {}\nrole = ' '",
            "session worker: activity needs at least one duration",
        ),
        (
            "empty watch precedes heartbeat interval",
            "[[session]]\nname = 'worker'\ncommand = ['cat']\nheartbeat = '0s'",
            "session worker: heartbeat needs a non-empty watch",
        ),
        (
            "session 1 role precedes session 2 name",
            "[[session]]\nname = 'worker'\ncommand = ['cat']\nrole = ' '\n[[session]]\nname = 'Bad'\ncommand = ['cat']",
            "session worker: role must not have leading or trailing whitespace",
        ),
    ];
    for (label, text, expected) in cases {
        assert_validation_error(label, text, expected);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dry_run_prints_exact_all_optional_details_in_order() {
    let (dir, _daemon) = common::start_daemon().await;
    std::fs::write(
        dir.path().join("dry-all.toml"),
        r#"
team = "t"
[[session]]
name = "worker"
command = ["cat"]
cwd = "/"
role = "developer"
attach = true
watch = ["other"]
control_from = ["other"]
heartbeat = "5s"
activity = { working_after = "2m", idle_after = "30s" }
[[session]]
name = "other"
command = ["cat"]
cwd = "/"
"#,
    )
    .unwrap();
    let output = run_binary(
        dir.path(),
        &["team", "up", "--dry-run", "--file", "dry-all.toml"],
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "would start t-worker\n  harness: generic\n  cwd: /\n  team: t, private, allow: -\n  role: developer\n  attach: true\n  watch: t-other\n  control_from: t-other\n  heartbeat: 5s\n  activity: working_after 2m, idle_after 30s\n  confinement: unconfined\n  command: cat\nwould start t-other\n  harness: generic\n  cwd: /\n  team: t, private, allow: -\n  confinement: unconfined\n  command: cat\n",
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dry_run_prints_working_only_activity_detail() {
    let (dir, _daemon) = common::start_daemon().await;
    std::fs::write(
        dir.path().join("dry-working.toml"),
        "[[session]]\nname = 'worker'\ncommand = ['cat']\nwatch = ['other']\nactivity = { working_after = '2m' }\n[[session]]\nname = 'other'\ncommand = ['cat']",
    )
    .unwrap();
    let output = run_binary(
        dir.path(),
        &["team", "up", "--dry-run", "--file", "dry-working.toml"],
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("  activity: working_after 2m\n")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dry_run_prints_idle_only_activity_detail() {
    let (dir, _daemon) = common::start_daemon().await;
    std::fs::write(
        dir.path().join("dry-idle.toml"),
        "[[session]]\nname = 'worker'\ncommand = ['cat']\nwatch = ['other']\nactivity = { idle_after = '30s' }\n[[session]]\nname = 'other'\ncommand = ['cat']",
    )
    .unwrap();
    let output = run_binary(
        dir.path(),
        &["team", "up", "--dry-run", "--file", "dry-idle.toml"],
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("  activity: idle_after 30s\n")
    );
}

#[test]
fn heartbeat_team_entry_requires_watch_and_validates_interval() {
    let parsed = team::parse(
        r#"
[[session]]
name = "architect"
command = ["sh"]
watch = ["developer"]
heartbeat = "30m"
"#,
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
    )
    .unwrap_err();
    assert!(format!("{no_watch:#}").contains("heartbeat needs a non-empty watch"));

    for value in ["0s", "25h", "30", "m", "1d", "-5m"] {
        let error = team::parse(&format!(
            "[[session]]\nname = \"architect\"\ncommand = [\"sh\"]\nwatch = [\"developer\"]\nheartbeat = {value:?}"
        ))
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
watch = ["developer"]
activity = { working_after = "2m", idle_after = "30s" }
"#,
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
            "[[session]]\nname = \"architect\"\ncommand = [\"sh\"]\nwatch = [\"developer\"]\nactivity = {}",
            "activity needs at least one duration",
        ),
        (
            "[[session]]\nname = \"architect\"\ncommand = [\"sh\"]\nwatch = [\"developer\"]\nactivity = { idle_after = \"25h\" }",
            "invalid activity idle_after",
        ),
        (
            "[[session]]\nname = \"architect\"\ncommand = [\"sh\"]\nwatch = [\"developer\"]\nactivity = { nope = \"30s\" }",
            "unknown field",
        ),
    ] {
        let error = team::parse(text).unwrap_err();
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
    )
    .unwrap_err();
    assert!(format!("{mutual:#}").contains("activity notices would loop between a and b"));

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
    )
    .unwrap_err();
    assert!(format!("{ring:#}").contains("activity notices would loop between a, b and c"));

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
grants = ["containers"]
write = ["~/.cache"]
[[session]]
name = "scratch"
command = ["omp"]
[session.confine]
enabled = false
"#,
    )
    .expect("new confine table");
    assert_eq!(
        sessions[0].confine,
        Some(a2amx::confine::ConfineRequest {
            write: vec!["~/.ctrscm".into(), "~/.cache".into()],
            hide: vec![],
            unhide: vec![],
            no_default_hides: false,
            grants: vec!["containers".into()],
            team_file: None,
        })
    );
    assert_eq!(sessions[1].confine, None);
}

#[test]
fn confine_table_rejects_old_keys_and_team_grants() {
    let old_inputs = [
        (
            "confine = false\n[[session]]\nname = 'worker'\ncommand = ['cat']",
            "confine is now a table: write [confine] with enabled = true; see docs/confinement.md",
        ),
        (
            "confine = true\n[[session]]\nname = 'worker'\ncommand = ['cat']",
            "confine is now a table: write [confine] with enabled = true; see docs/confinement.md",
        ),
        (
            "[[session]]\nname = 'worker'\ncommand = ['cat']\nconfine = false",
            "session 1: confine is now a table: use [session.confine] with enabled = true",
        ),
        (
            "[[session]]\nname = 'worker'\ncommand = ['cat']\nconfine_write = ['x']",
            "session 1: confine_write was replaced by write in [session.confine]",
        ),
        (
            "[[session]]\nname = 'worker'\ncommand = ['cat']\nconfine_hide = ['x']",
            "session 1: confine_hide was replaced by hide in [session.confine]",
        ),
        (
            "[[session]]\nname = 'worker'\ncommand = ['cat']\nconfine_unhide = ['x']",
            "session 1: confine_unhide was replaced by unhide in [session.confine]",
        ),
        (
            "[[session]]\nname = 'worker'\ncommand = ['cat']\nconfine_no_default_hides = true",
            "session 1: confine_no_default_hides was replaced by default_hides = false in [session.confine]",
        ),
        (
            "[[session]]\nname = 'worker'\ncommand = ['cat']\nconfine_allow_containers = true",
            "session 1: confine_allow_containers was replaced by grants = [\"containers\"] in [session.confine]",
        ),
    ];
    for (text, expected) in old_inputs {
        let error = team::parse(text).expect_err("old confinement syntax");
        assert_eq!(error.to_string(), expected);
    }

    let team_grant = team::parse(
        "[confine]\ngrants = ['containers']\n[[session]]\nname = 'worker'\ncommand = ['cat']",
    )
    .expect_err("team grant");
    assert!(team_grant.to_string().contains("grant it in the session"));
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
    )
    .expect_err("team shell-style path");
    assert!(
        team_error
            .to_string()
            .contains("[confine] write entry \"$HOME/.cache\": environment variables are not expanded; write the path out, or use ~/ for your home")
    );
}

#[test]
fn confine_table_requires_enabled_for_own_options_and_deduplicates_lists() {
    let error = team::parse(
        "[[session]]\nname = 'worker'\ncommand = ['cat']\n[session.confine]\nwrite = ['x']",
    )
    .expect_err("options without enabled");
    assert!(error.to_string().contains("need enabled = true"));

    let sessions = team::parse(
        "[confine]\nenabled = true\nwrite = ['x', 'x']\n[[session]]\nname = 'worker'\ncommand = ['cat']\n[session.confine]\nwrite = ['x', 'y', 'y']\ngrants = ['containers', 'containers']",
    )
    .expect("deduplicated confine lists");
    let request = sessions[0].confine.as_ref().expect("confined");
    assert_eq!(request.write, ["x", "y"]);
    assert_eq!(request.grants, ["containers"]);
}

#[test]
fn confine_table_covers_disabled_and_merged_limit_rules() {
    let inherited = team::parse(
        "[confine]\nenabled = false\nwrite = ['team']\n[[session]]\nname = 'worker'\ncommand = ['cat']\n[session.confine]\nenabled = true",
    )
    .expect("session enables team defaults");
    assert_eq!(
        inherited[0].confine.as_ref().expect("confined").write,
        ["team"]
    );
    let default_hides =
        team::parse("[confine]\nenabled = true\n[[session]]\nname = 'worker'\ncommand = ['cat']")
            .expect("default hides on");
    assert!(
        !default_hides[0]
            .confine
            .as_ref()
            .expect("confined")
            .no_default_hides
    );

    let ignored = team::parse(
        "[confine]\nwrite = ['team']\n[[session]]\nname = 'worker'\ncommand = ['cat']\n",
    )
    .expect("team options without enabled are ignored");
    assert_eq!(ignored[0].confine, None);

    let disabled = team::parse(
        "[confine]\nenabled = true\nwrite = ['team']\n[[session]]\nname = 'worker'\ncommand = ['cat']\n[session.confine]\nenabled = false",
    )
    .expect("session can opt out");
    assert_eq!(disabled[0].confine, None);

    let own_option = team::parse(
        "[confine]\nenabled = true\n[[session]]\nname = 'worker'\ncommand = ['cat']\n[session.confine]\nenabled = false\nwrite = ['own']",
    )
    .expect_err("disabled session options");
    assert!(own_option.to_string().contains("need enabled = true"));

    let unknown_key = team::parse(
        "[confine]\nenabled = true\nnope = true\n[[session]]\nname = 'worker'\ncommand = ['cat']",
    )
    .expect_err("unknown confine key");
    assert!(format!("{unknown_key:#}").contains("unknown field"));

    let unknown_grant = team::parse(
        "[[session]]\nname = 'worker'\ncommand = ['cat']\n[session.confine]\nenabled = true\ngrants = ['container']",
    )
    .expect_err("unknown session grant");
    assert!(
        unknown_grant
            .to_string()
            .contains("closest match: containers")
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
    ))
    .expect_err("merged write limit");
    assert!(
        merged_limit
            .to_string()
            .contains("session worker: write has at most 64 entries")
    );
    let long_path = "x".repeat(4097);
    let merged_bytes = team::parse(&format!(
        "[confine]\nenabled = true\nwrite = ['team']\n[[session]]\nname = 'worker'\ncommand = ['cat']\n[session.confine]\nwrite = ['{long_path}']"
    ))
    .expect_err("merged write byte limit");
    assert!(
        merged_bytes
            .to_string()
            .contains("write entry exceeds 4096 bytes")
    );
}
