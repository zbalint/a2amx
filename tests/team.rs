//! Team formats and lifecycle through the public module and real daemon.

mod common;

use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::Duration;

use common::pty::PtyHarness;

use a2amx::client::Client;
use a2amx::team::{self, TeamSession};
use a2amx::wire::{Request, Response, SessionSummary};

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
    }
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
                role: Some("architect".into()),
            },
            TeamSession {
                name: "developer".into(),
                command: vec!["omp".into()],
                cwd: Some(".".into()),
                attach: false,
                reset: None,
                control_from: Vec::new(),
                watch: Vec::new(),
                role: None,
                heartbeat: None,
            },
        ]
    );
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
prefix = "team"
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
prefix = "a2amx"
[[session]]
name = "architect"
command = ["claude"]
watch = ["developer", "outsider"]
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
    assert_eq!(sessions[0].watch, ["a2amx-developer", "outsider"]);
    assert_eq!(sessions[0].control_from, ["a2amx-developer"]);
    assert_eq!(sessions[1].control_from, ["a2amx-architect"]);
}

#[test]
fn parse_leaves_already_final_references_unchanged() {
    let sessions = team::parse(
        r#"
prefix = "a2amx"
[[session]]
name = "architect"
command = ["cat"]
watch = ["a2amx-developer"]
[[session]]
name = "developer"
command = ["cat"]
"#,
    )
    .unwrap();
    assert_eq!(sessions[0].watch, ["a2amx-developer"]);
}

#[test]
fn parse_base_name_match_wins_over_already_final_spelling() {
    let sessions = team::parse(
        r#"
prefix = "a2amx"
[[session]]
name = "architect"
command = ["cat"]
watch = ["a2amx-developer"]
control_from = ["a2amx-developer"]
[[session]]
name = "developer"
command = ["cat"]
[[session]]
name = "a2amx-developer"
command = ["cat"]
"#,
    )
    .unwrap();
    assert_eq!(sessions[0].watch, ["a2amx-a2amx-developer"]);
    assert_eq!(sessions[0].control_from, ["a2amx-a2amx-developer"]);
    assert_eq!(sessions[2].name, "a2amx-a2amx-developer");
}

#[test]
fn parse_rejects_empty_trailing_dash_and_non_string_prefixes() {
    for prefix in [r#""""#, r#""a2amx-""#, "5"] {
        let text =
            format!("prefix = {prefix}\n[[session]]\nname = \"architect\"\ncommand = [\"cat\"]");
        let error = team::parse(&text).expect_err(&text);
        assert!(format!("{error:#}").contains("prefix"), "{error:#}");
    }
}

#[test]
fn parse_validates_prefixed_final_names() {
    for (prefix, final_name) in [
        ("Bad", "Bad-architect"),
        (
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-architect",
        ),
    ] {
        let text = format!(
            "prefix = \"{prefix}\"\n[[session]]\nname = \"architect\"\ncommand = [\"cat\"]"
        );
        let error = team::parse(&text).expect_err(&text);
        assert!(format!("{error:#}").contains(final_name), "{error:#}");
    }
}

#[test]
fn parse_rejects_prefixed_self_watch() {
    let error = team::parse(
        "prefix = \"a2amx\"\n[[session]]\nname = \"developer\"\ncommand = [\"cat\"]\nwatch = [\"developer\"]",
    )
    .expect_err("prefixed self watch");
    assert!(error.to_string().contains("watches itself"), "{error:#}");
}

#[test]
fn parse_rejects_whitespace_in_executable_and_preserves_argument_boundaries() {
    for executable in ["claude --model opus", "/tmp/my executable", "cat\\t-n"] {
        let text = format!(
            "prefix = \"pfx\"\n[[session]]\nname = \"architect\"\ncommand = [\"{executable}\"]"
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
                role: None,
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
                role: None,
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
fn plan_preserves_wanted_order_and_preflights_every_exited_conflict() {
    let wanted = team::flag_sessions(&["architect=cat".into(), "developer=cat".into()]).unwrap();
    assert_eq!(
        team::plan(&wanted, &[summary("developer", "s2", None)]).unwrap(),
        [
            team::Action::Start(wanted[0].clone()),
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
            id: "s2".into()
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
                id: "s1".into()
            },
            team::Conflict {
                name: "developer".into(),
                id: "s2".into()
            },
        ],
    );
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
    assert!(listed.contains("s1  architect  generic  running  working   no"));
    assert!(listed.contains("s2  developer  generic  running  working   no"));
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
        format!("prefix = \"pfx\"\n{LIVE_TEAM}"),
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
