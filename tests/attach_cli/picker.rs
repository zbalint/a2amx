use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn picker_switches_sessions_and_exit_status_is_reported() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let (_, code) = run_cli(
        dir.path(),
        &[
            "new",
            "--detach",
            "--name",
            "aa",
            "--",
            "sh",
            "-c",
            "printf '%s%s' o ne; sleep 30",
        ],
    )?;
    assert_eq!(code, 0);
    let (_, code) = run_cli(
        dir.path(),
        &[
            "new",
            "--detach",
            "--name",
            "bb",
            "--",
            "sh",
            "-c",
            "printf '%s%s' t wo; sleep 30",
        ],
    )?;
    assert_eq!(code, 0);

    let mut blocker = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s2"]), dir.path())?;
    blocker.wait_for_text("two", WAIT)?;
    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("one", WAIT)?;

    // Entering and selecting the current row cancels and redraws the live session.
    attached.send(&[2, b'w'])?;
    attached.wait_for_text("a2amx sessions:", WAIT)?;
    for header in ["NAME", "HARNESS", "ACTIVITY", "PENDING", "HELD", "QUOTA"] {
        attached.wait_for_text(header, WAIT)?;
    }
    let screen = attached.screen_text();
    let running_row = screen
        .lines()
        .find(|line| line.starts_with(">s1 "))
        .expect("current session missing from picker");
    let cells: Vec<_> = running_row
        .trim_start_matches('>')
        .split_whitespace()
        .collect();
    assert_eq!(cells[0], "s1");
    assert_ne!(cells[1], "-");
    a2amx::messaging::validate_name(cells[1])?;
    assert_eq!(&cells[2..4], ["generic", "running"]);
    assert!(matches!(cells[4], "idle" | "working" | "busy"));
    attached.send(b"\r")?;
    attached.wait_for_text("one", WAIT)?;

    // A held controller makes the first switch fail; the picker remains open.
    attached.send(&[2, b'w'])?;
    attached.wait_for_text("a2amx sessions:", WAIT)?;
    attached.send(b"\x1b[B\r")?;
    attached.wait_for_text("attached elsewhere", WAIT)?;
    blocker.send(&[2, b'd'])?;
    blocker.wait_for_text("[detached from s2]", WAIT)?;
    blocker.wait_exit(WAIT)?;
    attached.send(b"\r")?;
    attached.wait_for_text("two", WAIT)?;
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s2]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);

    let (_, code) = run_cli(
        dir.path(),
        &["new", "--detach", "--", "sh", "-c", "printf bye; exit 7"],
    )?;
    assert_eq!(code, 0);
    let mut exited = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s3"]), dir.path())?;
    exited.wait_for_text("[session s3 exited with code 7]", WAIT)?;
    assert_eq!(exited.wait_exit(WAIT)?, 0);
    assert!(
        exited
            .raw_output()
            .windows(b"\x1b[?1049l".len())
            .any(|window| window == b"\x1b[?1049l")
    );

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("one", WAIT)?;
    attached.send(&[2, b'w'])?;
    attached.wait_for_text("exited(7)", WAIT)?;
    let screen = attached.screen_text();
    let exited_row = screen
        .lines()
        .find(|line| line.trim_start().starts_with("s3 "))
        .expect("exited session missing from picker");
    let cells: Vec<_> = exited_row.split_whitespace().collect();
    assert_eq!(cells[0], "s3");
    assert_ne!(cells[1], "-");
    a2amx::messaging::validate_name(cells[1])?;
    assert_eq!(&cells[2..5], ["generic", "exited(7)", "-"]);
    attached.send(b"\r")?;
    attached.wait_for_text("one", WAIT)?;
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn picker_scrolls_and_takes_page_home_end_keys() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    for name in [
        "pk-a", "pk-b", "pk-c", "pk-d", "pk-e", "pk-f", "pk-g", "pk-h",
    ] {
        let (_, code) = run_cli(
            dir.path(),
            &[
                "new", "--detach", "--name", name, "--", "sh", "-c", "sleep 30",
            ],
        )?;
        assert_eq!(code, 0);
    }

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "pk-a"]), dir.path())?;
    attached.resize(100, 8)?;
    attached.send(&[2, b'w'])?;
    attached.wait_for_text("1/8", WAIT)?;
    let screen = attached.screen_text();
    assert!(screen.contains("pk-a"), "picker screen: {screen:?}");
    assert!(!screen.contains("pk-h"), "picker screen: {screen:?}");

    attached.send(b"\x1b[F")?;
    attached.wait_for_text("8/8", WAIT)?;
    let screen = attached.screen_text();
    assert!(screen.contains("pk-h"), "picker screen: {screen:?}");
    assert!(!screen.contains("pk-a"), "picker screen: {screen:?}");

    attached.send(b"\x1b[H")?;
    attached.wait_for_text("1/8", WAIT)?;
    assert!(attached.screen_text().contains("pk-a"));
    attached.send(b"\x1b[6~")?;
    attached.wait_for_text("5/8", WAIT)?;

    attached.send(b"q")?;
    attached.wait_for_text("@host-a", WAIT)?;
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn picker_clips_long_cwd_to_narrow_terminal() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let cwd_root = tempfile::tempdir()?;
    let long_cwd = cwd_root
        .path()
        .join("fictional-picker-working-directory")
        .join("tail-of-picker-path");
    std::fs::create_dir_all(&long_cwd)?;

    let mut client = Client::connect(dir.path()).await?;
    let response = client
        .request(Request::NewSession {
            argv: vec![
                "sh".into(),
                "-c".into(),
                "stty size; trap 'stty size' WINCH; while :; do read line; done".into(),
            ],
            cols: 80,
            rows: 24,
            cwd: Some(long_cwd.to_string_lossy().into_owned()),
            env: vec![],
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            name: None,
            harness: Harness::Generic,
            deliver: None,
            heartbeat: None,
            activity: None,
            team: None,
            role: None,
            remote: None,
        })
        .await?;
    let Response::Created { session } = response else {
        anyhow::bail!("new session response was not Created");
    };

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", &session]), dir.path())?;
    attached.wait_for_text("22 80", WAIT)?;
    attached.resize(101, 10)?;
    attached.wait_for_text("8 101", WAIT)?;
    attached.send(&[2, b'w'])?;
    attached.wait_for_text("a2amx sessions:", WAIT)?;
    attached.wait_for_text("CWD", WAIT)?;
    let screen = attached.screen_text();
    assert!(screen.lines().any(|line| line.contains("CWD")));
    for line in screen.lines() {
        assert!(
            line.chars().count() <= 101,
            "picker screen line exceeded width: {line:?}"
        );
    }
    assert!(
        screen
            .lines()
            .any(|line| line.contains("…-picker-path (current)")),
        "picker did not left-shorten CWD: {screen:?}"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn picker_groups_sessions_into_team_sections() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let (created, code) = run_cli(
        dir.path(),
        &[
            "new", "--detach", "--name", "plain", "--", "sh", "-c", "sleep 30",
        ],
    )?;
    assert_eq!(code, 0);
    assert!(created.contains("s1"));
    std::fs::write(
        dir.path().join("config/team.toml"),
        "team = \"demo\"\n[[session]]\nname = \"worker\"\ncommand = [\"sh\", \"-c\", \"sleep 30\"]\n",
    )?;
    let team_file = dir.path().join("config/team.toml");
    let team_file = team_file.to_string_lossy().into_owned();
    let started = run_binary(
        dir.path(),
        &["team", "up", "--detach", "--file", &team_file],
        &[],
    )?;
    assert!(
        started.status.success(),
        "team up output: {}",
        String::from_utf8_lossy(&started.stderr)
    );

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.send(&[2, b'w'])?;
    attached.wait_for_text("a2amx sessions:", WAIT)?;
    attached.wait_for_text("NAME", WAIT)?;
    attached.wait_for_text("[-] demo (private) (1)", WAIT)?;
    attached.wait_for_text("worker", WAIT)?;
    attached.wait_for_text("[-] (no team) (1)", WAIT)?;
    let screen = attached.screen_text();
    assert!(!screen.contains("TEAM"), "picker screen: {screen:?}");
    assert!(screen.contains("plain"), "picker screen: {screen:?}");
    assert!(
        screen.find("NAME").unwrap_or(usize::MAX) < screen.find("worker").unwrap_or(0),
        "picker screen: {screen:?}"
    );
    attached.send(b"q")?;
    attached.wait_for_text("a2amx sessions:", WAIT)?;
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn picker_folds_teams_and_filters() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let (created, code) = run_cli(
        dir.path(),
        &[
            "new", "--detach", "--name", "base", "--", "sh", "-c", "sleep 30",
        ],
    )?;
    assert_eq!(code, 0);
    assert!(created.contains("s1"));
    for (team, sessions) in [
        ("alpha", [("al-1", "sleep 30"), ("al-2", "sleep 30")]),
        (
            "beta",
            [("be-1", "sleep 30"), ("be-2", "printf BE2-READY; sleep 30")],
        ),
        ("gamma", [("ga-1", "sleep 30"), ("ga-2", "sleep 30")]),
    ] {
        let path = dir.path().join(format!("{team}.toml"));
        let mut file = format!("team = \"{team}\"\n");
        for (name, command) in sessions {
            file.push_str(&format!(
                "[[session]]\nname = \"{name}\"\ncommand = [\"sh\", \"-c\", \"{command}\"]\n"
            ));
        }
        std::fs::write(&path, file)?;
        let path = path.to_string_lossy().into_owned();
        let started = run_binary(
            dir.path(),
            &["team", "up", "--detach", "--file", &path],
            &[],
        )?;
        assert!(
            started.status.success(),
            "team up {team} output: {}",
            String::from_utf8_lossy(&started.stderr)
        );
    }

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.send(&[2, b'w'])?;
    attached.wait_for_text("a2amx sessions:", WAIT)?;
    attached.wait_for_text("[+] alpha (private) (2)", WAIT)?;
    attached.wait_for_text("[+] beta (private) (2)", WAIT)?;
    attached.wait_for_text("[+] gamma (private) (2)", WAIT)?;
    attached.wait_for_text("[-] (no team) (1)", WAIT)?;
    attached.wait_for_text("base", WAIT)?;
    let screen = attached.screen_text();
    assert!(!screen.contains("al-1"), "picker screen: {screen:?}");

    attached.send(b"\x1b[H\r")?;
    attached.wait_for_text("[-] alpha (private) (2)", WAIT)?;
    attached.wait_for_text("al-1", WAIT)?;
    attached.wait_for_text("al-2", WAIT)?;
    let screen = attached.screen_text();
    assert!(!screen.contains("be-1"), "picker screen: {screen:?}");

    attached.send(b" ")?;
    attached.wait_for_text("[+] alpha (private) (2)", WAIT)?;
    let screen = attached.screen_text();
    assert!(!screen.contains("al-1"), "picker screen: {screen:?}");

    attached.send(b"/be-2")?;
    attached.wait_for_text("[-] beta (private) (1)", WAIT)?;
    attached.wait_for_text("be-2", WAIT)?;
    attached.wait_for_text("/be-2", WAIT)?;
    let screen = attached.screen_text();
    assert!(!screen.contains("al-1"), "picker screen: {screen:?}");

    attached.send(b"\r")?;
    attached.wait_for_text("BE2-READY", WAIT)?;
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from ", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    Ok(())
}
