use super::*;

#[test]
fn configured_reset_and_controller_are_accepted_by_team_parser() {
    let text = r#"
[[session]]
name = "worker"
command = ["cat"]
reset = ["/clear", "/prewalk restart"]
control_from = ["/architect"]
"#;
    let sessions = team::parse(text, std::path::Path::new("/work/demo"), false)
        .expect("valid reset configuration");
    assert_eq!(
        sessions[0].reset,
        Some(vec!["/clear".into(), "/prewalk restart".into()])
    );
    assert_eq!(sessions[0].control_from, ["architect"]);
    assert!(
        team::parse(
            &format!("{text}\nunknown = true"),
            Path::new("/work/proj"),
            false
        )
        .is_err()
    );
    let empty_reset = team::parse(
        "[[session]]\nname = \"worker\"\ncommand = [\"cat\"]\nreset = []",
        std::path::Path::new("/work/demo"),
        false,
    )
    .expect_err("empty reset must be rejected");
    assert!(empty_reset.to_string().contains("reset"));
    for reset in [
        "reset = [\"clear\"]",
        "reset = [\"/bad\\ncommand\"]",
        &format!("reset = [\"/{}\"]", "x".repeat(200)),
    ] {
        let invalid = format!("[[session]]\nname = \"worker\"\ncommand = [\"cat\"]\n{reset}");
        assert!(
            team::parse(&invalid, Path::new("/work/proj"), false).is_err(),
            "{reset}"
        );
    }
    let invalid_control =
        "[[session]]\nname = \"worker\"\ncommand = [\"cat\"]\ncontrol_from = [\"Bad_Name\"]";
    assert!(team::parse(invalid_control, Path::new("/work/proj"), false).is_err());
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
team = "team"
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
            session: "team-target".into(),
        })
        .await
        .unwrap();
    assert!(matches!(reset, Response::Reset { steps: 1 }), "{reset:?}");

    let mut other = Client::connect_addr(other_addr, &other_token)
        .await
        .unwrap();
    let denied = other
        .request(Request::Reset {
            session: "team-target".into(),
        })
        .await
        .unwrap();
    assert!(matches!(
        denied,
        Response::Failed { code, .. } if code == "not_permitted"
    ));
}

fn assert_validation_error(label: &str, text: &str, expected: &str) {
    let error = match team::parse(text, std::path::Path::new("/work/demo"), false) {
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
        "session 1 (demo-Bad): invalid name \"demo-Bad\"",
    );
}

#[test]
fn parse_characterizes_empty_command() {
    assert_validation_error(
        "empty command",
        "[[session]]\nname = 'worker'\ncommand = []",
        "session demo-worker: command must not be empty",
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
        "session demo-second: at most one session can attach",
    );
}

#[test]
fn parse_characterizes_empty_reset() {
    assert_validation_error(
        "empty reset",
        "[[session]]\nname = 'worker'\ncommand = ['cat']\nreset = []",
        "session demo-worker: reset must contain at least one step",
    );
}

#[test]
fn parse_characterizes_invalid_reset_step() {
    assert_validation_error(
        "invalid reset step",
        "[[session]]\nname = 'worker'\ncommand = ['cat']\nreset = ['not-a-control-key']",
        "session demo-worker: reset step 1 must start with '/'",
    );
}

#[test]
fn parse_characterizes_invalid_control_from() {
    assert_validation_error(
        "invalid control_from",
        "[[session]]\nname = 'worker'\ncommand = ['cat']\ncontrol_from = ['Bad']",
        "unknown session Bad in control_from of demo-worker",
    );
}

#[test]
fn parse_characterizes_invalid_watch() {
    assert_validation_error(
        "invalid watch",
        "[[session]]\nname = 'worker'\ncommand = ['cat']\nwatch = ['Bad']",
        "unknown session Bad in watch of demo-worker",
    );
}

#[test]
fn parse_characterizes_invalid_heartbeat() {
    assert_validation_error(
        "invalid heartbeat interval",
        "[[session]]\nname = 'worker'\ncommand = ['cat']\nwatch = ['/other']\nheartbeat = '0s'",
        "session demo-worker: invalid heartbeat",
    );
}

#[test]
fn parse_characterizes_invalid_role() {
    assert_validation_error(
        "invalid role",
        "[[session]]\nname = 'worker'\ncommand = ['cat']\nrole = ' '",
        "session demo-worker: role must not have leading or trailing whitespace",
    );
}

#[test]
fn parse_characterizes_check_precedence() {
    let cases = [
        (
            "duplicate precedes command",
            "[[session]]\nname = 'same'\ncommand = ['cat']\n[[session]]\nname = 'same'\ncommand = []",
            "session demo-same: duplicate name",
        ),
        (
            "command precedes attach",
            "[[session]]\nname = 'first'\ncommand = ['cat']\nattach = true\n[[session]]\nname = 'second'\ncommand = []\nattach = true",
            "session demo-second: command must not be empty",
        ),
        (
            "attach precedes reset",
            "[[session]]\nname = 'first'\ncommand = ['cat']\nattach = true\n[[session]]\nname = 'second'\ncommand = ['cat']\nattach = true\nreset = []",
            "session demo-second: at most one session can attach",
        ),
        (
            "reset precedes links",
            "[[session]]\nname = 'worker'\ncommand = ['cat']\nreset = []\ncontrol_from = ['Bad']",
            "unknown session Bad in control_from of demo-worker",
        ),
        (
            "watch precedes control_from",
            "[[session]]\nname = 'worker'\ncommand = ['cat']\ncontrol_from = ['Bad']\nwatch = ['Bad']\nheartbeat = '0s'",
            "unknown session Bad in watch of demo-worker",
        ),
        (
            "heartbeat precedes activity",
            "[[session]]\nname = 'worker'\ncommand = ['cat']\nwatch = ['/other']\nheartbeat = '0s'\nactivity = {}",
            "session demo-worker: invalid heartbeat",
        ),
        (
            "activity precedes role",
            "[[session]]\nname = 'worker'\ncommand = ['cat']\nwatch = ['/other']\nactivity = {}\nrole = ' '",
            "session demo-worker: activity needs at least one duration",
        ),
        (
            "empty watch precedes heartbeat interval",
            "[[session]]\nname = 'worker'\ncommand = ['cat']\nheartbeat = '0s'",
            "session demo-worker: heartbeat needs a non-empty watch",
        ),
        (
            "session 1 role precedes session 2 name",
            "[[session]]\nname = 'worker'\ncommand = ['cat']\nrole = ' '\n[[session]]\nname = 'Bad'\ncommand = ['cat']",
            "session demo-worker: role must not have leading or trailing whitespace",
        ),
    ];
    for (label, text, expected) in cases {
        assert_validation_error(label, text, expected);
    }
}
