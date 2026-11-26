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

const COMPOSER: &str = r#"import sys
sys.stdout.write("\x1b[?2004h")
sys.stdout.flush()
for raw in sys.stdin.buffer:
    line = raw.decode(errors="replace").replace("\x1b[200~", "").replace("\x1b[201~", "").rstrip("\r\n")
    if line == "/clear":
        print("marker:/clear", flush=True)
"#;

/// The token and address a session wrote to `path` from its own environment.
async fn read_credentials(path: std::path::PathBuf) -> (String, std::net::SocketAddr) {
    common::eventually(|| async {
        let text = std::fs::read_to_string(&path).ok()?;
        let mut lines = text.lines();
        let token = lines.next()?.to_owned();
        let address = lines.next()?.parse::<std::net::SocketAddr>().ok()?;
        (token.len() == 64).then_some((token, address))
    })
    .await
}

/// Wait until the reset gate would pass for `name`. The composer enables bracketed paste once
/// python has started, which can be after the credentials files exist; the admin connection is
/// the one that can explain a session of a private team.
async fn wait_until_reset_gate_passes(home: &Path, name: &str) {
    common::eventually(|| async {
        let mut admin = Client::connect(home).await.ok()?;
        let Response::Sessions { sessions } = admin.request(Request::List).await.ok()? else {
            return None;
        };
        let target = sessions
            .into_iter()
            .find(|session| session.name.as_deref() == Some(name))?;
        let Response::Explain { info } = admin
            .request(Request::Explain { session: target.id })
            .await
            .ok()?
        else {
            return None;
        };
        info.reset.allowed.then_some(())
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn team_up_forwards_reset_configuration_and_consent() {
    let (dir, _daemon) = common::start_daemon().await;
    let controller_file = dir.path().join("controller.creds");
    let other_file = dir.path().join("other.creds");
    let composer = dir.path().join("team-composer.py");
    std::fs::write(&composer, COMPOSER).unwrap();
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
reset = ["/clear", "/clear"]
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

    let (controller_token, controller_addr) = read_credentials(controller_file).await;
    let (other_token, other_addr) = read_credentials(other_file).await;
    let mut controller = Client::connect_addr(controller_addr, &controller_token)
        .await
        .unwrap();
    // The composer enables bracketed paste once python has started, which can be after the
    // controller wrote its credentials: wait until the reset gate would pass, then reset once.
    wait_until_reset_gate_passes(dir.path(), "team-target").await;
    let reset = controller
        .request(Request::Reset {
            session: "team-target".into(),
        })
        .await
        .unwrap();
    assert!(matches!(reset, Response::Reset { steps: 2 }), "{reset:?}");

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

#[test]
fn merge_confine_check_precedence_and_disabled_shortcut_are_exact() {
    let dir = Path::new("/work/demo");
    let many_writes = (0..65)
        .map(|i| format!("\"/tmp/w{i}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let many_grants = (0..65)
        .map(|i| format!("\"bogus{i}\""))
        .collect::<Vec<_>>()
        .join(", ");

    let cases = [
        (
            "C2 precedes C3",
            "[confine]\nauto = true\nenabled = false\n[[session]]\nname = 'worker'\ncommand = ['cat']\n[session.confine]\nauto = true",
            "[confine] auto needs confinement; remove enabled = false",
        ),
        (
            "C3 precedes C4",
            "[confine]\nkeep_env = ['A']\n[[session]]\nname = 'worker'\ncommand = ['cat']\n[session.confine]\nauto = false",
            "session worker: auto belongs in the team's [confine] table, not [session.confine]",
        ),
        (
            "C4 precedes C5",
            "[confine]\nkeep_env = ['A']\n[[session]]\nname = 'worker'\ncommand = ['cat']\n[session.confine]\nwrite = ['']",
            "[confine] keep_env: a credential variable is returned per session, not team-wide; put keep_env in [session.confine]",
        ),
        (
            "C5 precedes C6",
            "[confine]\ngrants = ['snap']\n[[session]]\nname = 'worker'\ncommand = ['cat']\n[session.confine]\nwrite = ['']",
            "session worker: write entry is empty",
        ),
        (
            "C6 precedes C7",
            "[confine]\ngrants = ['snap']\n[[session]]\nname = 'worker'\ncommand = ['cat']\n[session.confine]\nenabled = false\nwrite = ['/tmp']",
            "[confine] grants: \"snap\" exposes snapd package management: installs and removes packages; grant it in the session that needs it, not team-wide",
        ),
        (
            "C7 precedes C9",
            &format!(
                "[[session]]\nname = 'worker'\ncommand = ['cat']\n[session.confine]\nenabled = false\nwrite = [{many_writes}]"
            ),
            "session worker: [confine] options are set but enabled = false; remove the options or the enabled = false",
        ),
        (
            "C7 sees hide alone",
            "[[session]]\nname = 'worker'\ncommand = ['cat']\n[session.confine]\nenabled = false\nhide = ['/x']",
            "session worker: [confine] options are set but enabled = false; remove the options or the enabled = false",
        ),
        (
            "C7 sees unhide alone",
            "[[session]]\nname = 'worker'\ncommand = ['cat']\n[session.confine]\nenabled = false\nunhide = ['/x']",
            "session worker: [confine] options are set but enabled = false; remove the options or the enabled = false",
        ),
        (
            "C7 sees grants alone",
            "[[session]]\nname = 'worker'\ncommand = ['cat']\n[session.confine]\nenabled = false\ngrants = ['snap']",
            "session worker: [confine] options are set but enabled = false; remove the options or the enabled = false",
        ),
        (
            "C7 sees keep_env alone",
            "[[session]]\nname = 'worker'\ncommand = ['cat']\n[session.confine]\nenabled = false\nkeep_env = ['A']",
            "session worker: [confine] options are set but enabled = false; remove the options or the enabled = false",
        ),
        (
            "C7 sees default_hides alone",
            "[[session]]\nname = 'worker'\ncommand = ['cat']\n[session.confine]\nenabled = false\ndefault_hides = false",
            "session worker: [confine] options are set but enabled = false; remove the options or the enabled = false",
        ),
        (
            "C9 precedes C10",
            &format!(
                "[[session]]\nname = 'worker'\ncommand = ['cat']\n[session.confine]\ngrants = [{many_grants}]"
            ),
            "session worker: grants has at most 64 entries",
        ),
    ];
    for (label, text, expected) in cases {
        let err = team::parse(text, dir, false).expect_err(label);
        assert_eq!(err.to_string(), expected, "{label}");
    }

    let both = "[confine]\ndefault_hides = false\n[[session]]\nname = 'worker'\ncommand = ['cat']\n[session.confine]\ndefault_hides = true";
    let sessions = team::parse(both, dir, false).expect("own default_hides wins");
    assert!(
        !sessions[0]
            .confine
            .as_ref()
            .expect("confine")
            .no_default_hides
    );

    let disabled_team = format!(
        "[confine]\nenabled = false\nwrite = [{many_writes}]\n[[session]]\nname = 'worker'\ncommand = ['cat']"
    );
    let sessions = team::parse(&disabled_team, dir, false).expect("C8 returns before C9");
    assert_eq!(sessions[0].confine, None);
}

#[test]
fn allow_agents_and_references_characterize_exact_errors() {
    let dir = Path::new("/work/a2amx");
    let allow_cases = [
        (
            "saltmdb",
            "session a2amx-worker: allow_agents entry \"saltmdb\" must be TEAM/AGENT",
        ),
        (
            "/x",
            "session a2amx-worker: allow_agents entry \"/x\" must be TEAM/AGENT",
        ),
        (
            "saltmdb/",
            "session a2amx-worker: allow_agents entry \"saltmdb/\" must be TEAM/AGENT",
        ),
        (
            "a/b/c",
            "session a2amx-worker: allow_agents entry \"a/b/c\" must be TEAM/AGENT",
        ),
        (
            "Bad/x",
            "session a2amx-worker: allow_agents entry \"Bad/x\": invalid name \"Bad\": expected [a-z0-9][a-z0-9-]{0,62}",
        ),
        (
            "x/Bad",
            "session a2amx-worker: allow_agents entry \"x/Bad\": invalid name \"Bad\": expected [a-z0-9][a-z0-9-]{0,62}",
        ),
        (
            "a2amx/x",
            "session a2amx-worker: allow_agents entry \"a2amx/x\" must name another team",
        ),
    ];
    for (entry, expected) in allow_cases {
        let text =
            format!("[[session]]\nname = 'worker'\ncommand = ['cat']\nallow_agents = ['{entry}']");
        let err = team::parse(&text, dir, false).expect_err(entry);
        assert_eq!(err.to_string(), expected, "{entry}");
    }

    let repeated = "[[session]]\nname = 'worker'\ncommand = ['cat']\nallow_agents = ['saltmdb/a', 'saltmdb/a']";
    assert_eq!(
        team::parse(repeated, dir, false).unwrap_err().to_string(),
        "session a2amx-worker: allow_agents entry \"saltmdb/a\" is repeated"
    );

    let sixteen = (0..16)
        .map(|i| format!("'saltmdb/a{i}'"))
        .collect::<Vec<_>>()
        .join(", ");
    assert!(
        team::parse(
            &format!("[[session]]\nname = 'worker'\ncommand = ['cat']\nallow_agents = [{sixteen}]"),
            dir,
            false,
        )
        .is_ok()
    );

    let seventeen_bad = (0..17)
        .map(|i| format!("'bad{i}'"))
        .collect::<Vec<_>>()
        .join(", ");
    assert_eq!(
        team::parse(
            &format!(
                "private = false\n[[session]]\nname = 'worker'\ncommand = ['cat']\nallow_agents = [{seventeen_bad}]"
            ),
            dir,
            false,
        )
        .unwrap_err()
        .to_string(),
        "session a2amx-worker: allow_agents needs a private team"
    );
    assert_eq!(
        team::parse(
            &format!(
                "[[session]]\nname = 'worker'\ncommand = ['cat']\nallow_agents = [{seventeen_bad}]"
            ),
            dir,
            false,
        )
        .unwrap_err()
        .to_string(),
        "session a2amx-worker: allow_agents has at most 16 entries"
    );

    assert_eq!(
        team::parse(
            "[[session]]\nname = 'worker'\ncommand = ['cat']\nallow_agents = ['a2amx/daemon']",
            Path::new("/work/demo"),
            false,
        )
        .unwrap_err()
        .to_string(),
        "session demo-worker: allow_agents entry \"a2amx/daemon\": invalid name \"a2amx-daemon\": reserved for the daemon"
    );

    for (reference, expected) in [
        (
            "/Bad",
            "invalid name \"Bad\": expected [a-z0-9][a-z0-9-]{0,62}",
        ),
        (
            "Bad/x",
            "invalid name \"Bad\": expected [a-z0-9][a-z0-9-]{0,62}",
        ),
        (
            "x/Bad",
            "invalid name \"Bad\": expected [a-z0-9][a-z0-9-]{0,62}",
        ),
    ] {
        let text =
            format!("[[session]]\nname = 'architect'\ncommand = ['cat']\nwatch = ['{reference}']");
        assert_eq!(
            team::parse(&text, Path::new("/work/demo"), false)
                .unwrap_err()
                .to_string(),
            expected,
            "{reference}"
        );
    }
    assert_eq!(
        team::parse(
            "[[session]]\nname = 'architect'\ncommand = ['cat']\nwatch = ['a2amx/daemon']",
            Path::new("/work/demo"),
            false,
        )
        .unwrap_err()
        .to_string(),
        "session demo-architect watch entry \"a2amx/daemon\": invalid name \"a2amx-daemon\": reserved for the daemon"
    );
}
