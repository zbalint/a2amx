use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_line_toggles_and_shows_the_session_address() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let script = "stty size; trap 'stty size' WINCH; while :; do read line; done";
    let (_, code) = run_cli(
        dir.path(),
        &[
            "new",
            "--detach",
            "--name",
            "agent-plan",
            "--",
            "sh",
            "-c",
            script,
        ],
    )?;
    assert_eq!(code, 0);
    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("22 80", WAIT)?;
    attached.wait_for_text("agent-plan@", WAIT)?;
    assert!(
        attached
            .screen_text()
            .lines()
            .last()
            .unwrap_or("")
            .contains("agent-plan@")
    );
    attached.resize(40, 10)?;
    attached.wait_for_text("8 40", WAIT)?;
    let after_toggle = attached.raw_output().len();
    attached.send(&[2, b's'])?;
    if let Err(error) = attached.wait_for_text("10 40", WAIT) {
        eprintln!(
            "post-toggle outer PTY bytes: {:?}",
            String::from_utf8_lossy(&attached.raw_output()[after_toggle..]),
        );
        return Err(error);
    }
    assert!(
        !attached
            .screen_text()
            .lines()
            .last()
            .unwrap_or("")
            .contains("agent-plan@")
    );
    attached.resize(50, 12)?;
    attached.wait_for_text("12 50", WAIT)?;
    attached.send(&[2, b's'])?;
    attached.wait_for_text("10 50", WAIT)?;
    assert!(
        attached
            .screen_text()
            .lines()
            .last()
            .unwrap_or("")
            .contains("agent-plan@")
    );
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_separator_keeps_session_output_above_the_bar() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let script = "read trigger; set -- $(stty size); i=1; while [ $i -le $1 ]; do printf '\\033[%s;1Hsession-row-%02d' \"$i\" \"$i\"; i=$((i+1)); done; sleep 30";
    let (_, code) = run_cli(dir.path(), &["new", "--detach", "--", "sh", "-c", script])?;
    assert_eq!(code, 0);
    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("@host-a", WAIT)?;
    attached.send(b"draw\n")?;
    attached.wait_for_text("session-row-22", WAIT)?;
    let screen = attached.screen_text();
    let rows: Vec<_> = screen.lines().collect();
    assert_eq!(rows.len(), 24, "screen: {screen:?}");
    assert!(rows[23].contains("@host-a"), "screen: {screen:?}");
    assert_eq!(rows[22], "─".repeat(79), "screen: {screen:?}");
    for (row, marker) in rows[..22].iter().zip([
        "session-row-01",
        "session-row-02",
        "session-row-03",
        "session-row-04",
        "session-row-05",
        "session-row-06",
        "session-row-07",
        "session-row-08",
        "session-row-09",
        "session-row-10",
        "session-row-11",
        "session-row-12",
        "session-row-13",
        "session-row-14",
        "session-row-15",
        "session-row-16",
        "session-row-17",
        "session-row-18",
        "session-row-19",
        "session-row-20",
        "session-row-21",
        "session-row-22",
    ]) {
        assert_eq!(*row, marker, "screen: {screen:?}");
    }
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn team_line_renders_clicks_switches_and_toggles_for_real_attach() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let team_file = dir.path().join("demo.toml");
    std::fs::write(
        &team_file,
        "team = \"demo\"\n[[session]]\nname = \"alpha\"\ncommand = [\"sh\", \"-c\", \"printf '\\\\033[?1000h\\\\033[?1006h'; printf ALPHA; stty raw -echo; cat -v\"]\n[[session]]\nname = \"beta\"\ncommand = [\"sh\", \"-c\", \"printf BETA; sleep 30\"]\n[[session]]\nname = \"dead\"\ncommand = [\"sh\", \"-c\", \"exit 0\"]\n",
    )?;
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

    let small_command = format!(
        "stty rows 4; exec {} attach demo-alpha",
        env!("CARGO_BIN_EXE_a2amx")
    );
    let small_args = vec!["-c".to_owned(), small_command];
    let mut small = PtyHarness::spawn_program("sh", &small_args, dir.path(), &[])?;
    small.wait_for_text("ALPHA", WAIT)?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let small_screen = small.screen_text();
    assert!(
        !small_screen
            .lines()
            .next()
            .unwrap_or_default()
            .contains("team demo"),
        "small screen: {small_screen:?}"
    );
    small.send(&[2, b'd'])?;
    small.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(small.wait_exit(WAIT)?, 0);

    for (rows, draws_rule) in [(5, false), (6, true)] {
        let command = format!(
            "stty rows {rows}; exec {} attach demo-alpha",
            env!("CARGO_BIN_EXE_a2amx")
        );
        let args = vec!["-c".to_owned(), command];
        let mut sized = PtyHarness::spawn_program("sh", &args, dir.path(), &[])?;
        sized.wait_for_text("ALPHA", WAIT)?;
        sized.wait_for_text("team demo", WAIT)?;
        let lines = sized.screen_text();
        let rows = lines.lines().collect::<Vec<_>>();
        assert!(rows[0].contains("team demo"), "screen: {lines:?}");
        if draws_rule {
            assert_eq!(rows[1], "─".repeat(79), "screen: {lines:?}");
            assert!(rows[2].contains("ALPHA"), "screen: {lines:?}");
        } else {
            assert_ne!(rows[1], "─".repeat(79), "screen: {lines:?}");
        }
        sized.send(&[2, b'd'])?;
        sized.wait_for_text("[detached from s1]", WAIT)?;
        assert_eq!(sized.wait_exit(WAIT)?, 0);
    }

    let mut attached =
        PtyHarness::spawn(&cli_args(dir.path(), &["attach", "demo-alpha"]), dir.path())?;
    attached.wait_for_text("ALPHA", WAIT)?;
    attached.wait_for_text("team demo", WAIT)?;
    let initial_screen = attached.screen_text();
    let initial_rows = initial_screen.lines().collect::<Vec<_>>();
    assert_eq!(
        initial_rows[1],
        "─".repeat(79),
        "screen: {initial_screen:?}"
    );
    assert!(
        initial_rows[2].contains("ALPHA"),
        "screen: {initial_screen:?}"
    );
    assert!(attached.screen_text().contains("demo-alpha"));
    attached.wait_for_text("demo-beta", WAIT)?;
    attached.wait_for_text("demo-dead", WAIT)?;
    attached.send(&[2, b'3'])?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(attached.screen_text().contains("demo-alpha@host-a"));

    attached.send(b"\x1b[<0;30;1M\x1b[<0;30;1m")?;
    attached.wait_for_text("BETA", WAIT)?;
    attached.wait_for_text("demo-beta@host-a", WAIT)?;

    attached.send(&[2, b'1'])?;
    attached.wait_for_text("ALPHA", WAIT)?;
    attached.wait_for_text("demo-alpha@host-a", WAIT)?;
    attached.send(b"\x1b[M\x20\x3e\x21\x1b[M\x23\x3e\x21")?;
    attached.wait_for_text("BETA", WAIT)?;
    attached.wait_for_text("demo-beta@host-a", WAIT)?;
    attached.send(&[2, b'1'])?;
    attached.wait_for_text("ALPHA", WAIT)?;
    attached.wait_for_text("demo-alpha@host-a", WAIT)?;
    attached.send(b"\x1b[<0;18;1M\x1b[<0;18;1m")?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(attached.screen_text().contains("demo-alpha@host-a"));

    attached.send(b"\x1b[<0;18;1M\x1b[<0;10;2m")?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let team_to_rule = attached.screen_text();
    assert!(!team_to_rule.contains("^[[<0;10;1m"));
    assert!(
        !team_to_rule.contains("a2amx sessions:"),
        "screen: {team_to_rule:?}"
    );

    attached.send(b"\x1b[<0;5;2M\x1b[<0;5;1m")?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let rule_to_team = attached.screen_text();
    assert!(!rule_to_team.contains("^[[<0;5;1m"));
    assert!(
        !rule_to_team.contains("a2amx sessions:"),
        "screen: {rule_to_team:?}"
    );

    attached.send(b"\x1b[<0;12;2M\x1b[<0;12;2m")?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let rule_click = attached.screen_text();
    assert!(!rule_click.contains("^[[<0;12;2m"));
    assert!(
        !rule_click.contains("a2amx sessions:"),
        "screen: {rule_click:?}"
    );

    attached.send(b"\x1b[<0;10;3M\x1b[<0;10;1m")?;
    attached.wait_for_text("^[[<0;10;1m", WAIT)?;
    attached.send(b"\x1b[<0;11;3M\x1b[<64;11;2M\x1b[<0;11;3m")?;
    attached.wait_for_text("^[[<0;11;1m", WAIT)?;

    attached.send(&[2, b'2'])?;
    attached.wait_for_text("BETA", WAIT)?;
    attached.wait_for_text("demo-beta@host-a", WAIT)?;
    attached.send(&[2, b'w'])?;
    attached.wait_for_text("a2amx sessions:", WAIT)?;
    attached.send(b"\x1b[<64;18;1M\x1b[<64;18;1m")?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    attached.send(b"\x1b[<0;18;1M\x1b[<0;18;1m")?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    attached.send(b"q")?;
    attached.wait_for_text("demo-beta@host-a", WAIT)?;
    attached.send(&[2, b'['])?;
    attached.send(b"\x1b[<0;18;1M\x1b[<0;18;1m")?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    attached.send(b"q")?;
    attached.wait_for_text("demo-beta@host-a", WAIT)?;

    attached.send(&[2, b's'])?;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let hidden_screen = attached.screen_text();
    let hidden_first_line = hidden_screen.lines().next().unwrap_or_default();
    assert!(
        hidden_first_line.contains("BETA"),
        "hidden screen: {hidden_screen:?}"
    );
    assert!(
        !hidden_first_line.contains("team demo"),
        "hidden screen: {hidden_screen:?}"
    );
    assert_ne!(
        hidden_screen.lines().nth(1).unwrap_or_default(),
        "─".repeat(79),
        "hidden screen: {hidden_screen:?}"
    );
    attached.send(&[2, b's'])?;
    attached.wait_for_text("team demo", WAIT)?;
    let restored_screen = attached.screen_text();
    assert_eq!(
        restored_screen.lines().nth(1).unwrap_or_default(),
        "─".repeat(79),
        "restored screen: {restored_screen:?}"
    );

    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s2]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);

    let (created, code) = run_cli(
        dir.path(),
        &[
            "new",
            "--detach",
            "--name",
            "plain",
            "--",
            "sh",
            "-c",
            "printf PLAIN; sleep 30",
        ],
    )?;
    assert_eq!(code, 0);
    assert!(!created.is_empty());
    let mut plain = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "plain"]), dir.path())?;
    plain.wait_for_text("PLAIN", WAIT)?;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!plain.screen_text().contains("team demo"));
    plain.send(&[2, b'd'])?;
    plain.wait_for_text("[detached from s4]", WAIT)?;
    assert_eq!(plain.wait_exit(WAIT)?, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_toggle_clears_team_row_without_session_output() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let team_file = dir.path().join("silent.toml");
    std::fs::write(
        &team_file,
        "team = \"silent\"\n[[session]]\nname = \"alpha\"\ncommand = [\"sh\", \"-c\", \"sleep 30\"]\n",
    )?;
    let team_file = team_file.to_string_lossy().into_owned();
    let started = run_binary(
        dir.path(),
        &["team", "up", "--detach", "--file", &team_file],
        &[],
    )?;
    assert!(started.status.success());

    let mut attached = PtyHarness::spawn(
        &cli_args(dir.path(), &["attach", "silent-alpha"]),
        dir.path(),
    )?;
    attached.wait_for_text("team silent", WAIT)?;
    attached.send(&[2, b's'])?;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let hidden = attached.screen_text();
    assert!(
        !hidden
            .lines()
            .next()
            .unwrap_or_default()
            .contains("team silent"),
        "hidden screen: {hidden:?}"
    );
    assert_ne!(
        hidden.lines().nth(1).unwrap_or_default(),
        "─".repeat(79),
        "hidden screen: {hidden:?}"
    );
    let clear_rule = b"\x1b7\x1b[2;1H\x1b[0m\x1b[2K\x1b8";
    assert!(
        attached
            .raw_output()
            .windows(clear_rule.len())
            .any(|window| window == clear_rule),
        "raw output did not clear row 2: {hidden:?}"
    );
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_toggle_reattaches_to_resize_the_session() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    std::fs::create_dir_all(dir.path().join("run"))?;
    std::fs::write(dir.path().join("run/addr"), format!("{address}\n"))?;
    std::fs::write(dir.path().join("run/admin.token"), "fake-token\n")?;
    let daemon = FakeDaemon::new(fake_team());
    let server = tokio::spawn(run_fake_daemon(listener, daemon.clone()));

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("alpha@host-a", WAIT)?;
    attached.send(&[2, b's'])?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(daemon.attach_count.load(Ordering::SeqCst), 2);
    attached.send(&[2, b'd'])?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    server.abort();
    let _ = server.await;
    Ok(())
}
