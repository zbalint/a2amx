use super::*;

const LIVE_TEAM: &str = r#"
[[session]]
name = "architect"
command = ["sh", "-c", "sleep 30"]
[[session]]
name = "developer"
command = ["sh", "-c", "sleep 30"]
"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_notes_messages_lost_in_a_daemon_restart_only_for_wanted_names() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon_in(dir.path(), Some("host-a"), Default::default()).await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_recipient_id, _recipient_token, _recipient_addr) = common::new_agent(
        &mut admin,
        dir.path(),
        Some("wanted-wanted"),
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
            to: "wanted-wanted@host-a".into(),
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
        "team = \"wanted\"\n[[session]]\nname = 'wanted'\ncommand = ['cat']",
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
    std::fs::write(dir.path().join("config/team.toml"), LIVE_TEAM).unwrap();
    let first = run_binary(
        dir.path(),
        &["team", "up", "--detach", "--file", "config/team.toml"],
    )
    .unwrap();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(
        first.stdout,
        b"started config-architect s1\nstarted config-developer s2\n"
    );
    let listed = run_binary(dir.path(), &["list"]).unwrap();
    let listed = String::from_utf8(listed.stdout).unwrap();
    for (id, name) in [("s1", "config-architect"), ("s2", "config-developer")] {
        let cells = common::list_cells(&listed, id);
        assert_eq!(&cells[..5], [id, name, "config", "(private)", "generic"]);
        assert_eq!(cells[8], "working");
        assert_eq!(cells[10], "no");
        for index in [7, 9] {
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
    let second = run_binary(dir.path(), &["team", "up", "--file", "config/team.toml"]).unwrap();
    assert!(second.status.success());
    assert_eq!(
        second.stdout,
        b"already running config-architect s1 (confine off)\nalready running config-developer s2 (confine off)\n"
    );
    let listed_again = run_binary(dir.path(), &["list"]).unwrap();
    assert_eq!(listed_again.stdout, listed.as_bytes());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn down_kills_only_named_team_sessions_and_reports_missing_names() {
    let (dir, _daemon) = common::start_daemon().await;
    std::fs::write(dir.path().join("config/team.toml"), LIVE_TEAM).unwrap();
    assert!(
        run_binary(
            dir.path(),
            &["team", "up", "--detach", "--file", "config/team.toml"]
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
    let down = run_binary(dir.path(), &["team", "down", "--file", "config/team.toml"]).unwrap();
    assert!(
        down.status.success(),
        "{}",
        String::from_utf8_lossy(&down.stderr)
    );
    assert_eq!(
        down.stdout,
        b"killed config-architect s1\nkilled config-developer s2\n"
    );
    let repeated = run_binary(dir.path(), &["team", "down", "--file", "config/team.toml"]).unwrap();
    assert!(repeated.status.success());
    assert_eq!(
        repeated.stdout,
        b"no session config-architect\nno session config-developer\n"
    );
    let listed = String::from_utf8(run_binary(dir.path(), &["list"]).unwrap().stdout).unwrap();
    let cells = common::list_cells(&listed, "s3");
    assert_eq!(
        &cells[..5],
        ["s3", "unrelated", "generic", "off", "running"]
    );
    assert!(!listed.contains("config-architect") && !listed.contains("config-developer"));
    let named = run_binary(dir.path(), &["team", "down", "unrelated", "missing"]).unwrap();
    assert!(named.status.success());
    assert_eq!(named.stdout, b"killed unrelated s3\nno session missing\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prefixed_team_down_uses_full_names_and_hints_only_for_explicit_missing_names() {
    let (dir, _daemon) = common::start_daemon().await;
    std::fs::write(
        dir.path().join("config/team.toml"),
        format!("team = \"pfx\"\n{LIVE_TEAM}"),
    )
    .unwrap();
    let up = run_binary(
        dir.path(),
        &["team", "up", "--detach", "--file", "config/team.toml"],
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
    let down = run_binary(dir.path(), &["team", "down", "--file", "config/team.toml"]).unwrap();
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
    let repeated = run_binary(dir.path(), &["team", "down", "--file", "config/team.toml"]).unwrap();
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
                "new",
                "--detach",
                "--name",
                "local-exited",
                "--",
                "sh",
                "-c",
                "exit 0"
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
team = "local"
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
            .contains("session local-exited has exited (s1); run a2amx kill local-exited first")
    );
    let listed = String::from_utf8(run_binary(dir.path(), &["list"]).unwrap().stdout).unwrap();
    assert!(listed.contains("s1  local-exited"));
    assert!(!listed.contains("missing") && !listed.contains("s2"));
    let down = run_binary(dir.path(), &["team", "down"]).unwrap();
    assert!(down.status.success());
    assert_eq!(
        down.stdout,
        b"no session local-missing\nkilled local-exited s1\n"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn first_spawn_failure_keeps_started_sessions_and_stops_the_team() {
    let (dir, _daemon) = common::start_daemon().await;
    std::fs::write(
        dir.path().join("a2amx.toml"),
        r#"
team = "local"
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
    assert_eq!(output.stdout, b"started local-started s1\n");
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("failed local-broken:")
    );
    let listed = String::from_utf8(run_binary(dir.path(), &["list"]).unwrap().stdout).unwrap();
    let cells = common::list_cells(&listed, "s1");
    assert_eq!(
        &cells[..5],
        ["s1", "local-started", "local", "(private)", "generic"]
    );
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
    let architect = common::list_cells(&listed, "s1");
    let developer = common::list_cells(&listed, "s2");
    assert_eq!(
        &architect[..5],
        ["s1", "architect", "generic", "off", "running"]
    );
    assert_eq!(
        &developer[..5],
        ["s2", "developer", "generic", "off", "running"]
    );
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
        dir.path().join("config/team.toml"),
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
    let team_file = dir
        .path()
        .join("config/team.toml")
        .to_string_lossy()
        .into_owned();
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
    first.wait_for_text("architect@host-a", wait)?;
    first.send(&[2, b'd'])?;
    first.wait_for_text("[detached from s3]", wait)?;
    assert_eq!(first.wait_exit(wait)?, 0);

    let mut conflict = PtyHarness::spawn(
        &args(&["team", "up", "config-architect=cat", "config-developer=cat"]),
        dir.path(),
    )?;
    assert_eq!(conflict.wait_exit(wait)?, 1);
    assert!(conflict.screen_text().contains("different team setting"));

    let mut detached = PtyHarness::spawn(
        &args(&["team", "up", "--detach", "--file", &team_file]),
        dir.path(),
    )?;
    assert_eq!(detached.wait_exit(wait)?, 0);
    assert!(
        detached
            .screen_text()
            .contains("already running config-architect s1")
    );
    assert!(
        detached
            .screen_text()
            .contains("already running config-developer s2")
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn team_up_and_down_characterize_exact_conflict_abort_and_missing_outputs() {
    let (dir, _daemon) = common::start_daemon().await;
    let file = dir.path().join("config/team.toml");
    let file_arg = file.to_str().unwrap();
    std::fs::write(
        &file,
        "team = \"demo\"\n[[session]]\nname = \"alpha\"\ncommand = [\"true\"]\n",
    )
    .unwrap();
    assert!(
        run_binary(dir.path(), &["team", "up", "--detach", "--file", file_arg])
            .unwrap()
            .status
            .success()
    );

    // Wait for alpha to exit, then re-run `team up` to characterize exact conflict stderr.
    let mut waited = 0;
    loop {
        let status = run_binary(dir.path(), &["team", "status", "--file", file_arg]).unwrap();
        if String::from_utf8_lossy(&status.stdout).contains("exited") {
            break;
        }
        assert!(waited < 50);
        waited += 1;
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let conflict = run_binary(dir.path(), &["team", "up", "--detach", "--file", file_arg]).unwrap();
    assert!(!conflict.status.success());
    assert_eq!(
        String::from_utf8(conflict.stderr).unwrap(),
        "session demo-alpha has exited (s1); run a2amx kill demo-alpha first\na2amx: team has session conflicts\n"
    );

    // `team down` with missing explicit name prints `no session`, hint, and exits 0.
    let down_missing = run_binary(dir.path(), &["team", "down", "no-such"]).unwrap();
    assert!(down_missing.status.success());
    assert_eq!(
        String::from_utf8(down_missing.stdout).unwrap(),
        "no session no-such\n"
    );
    assert_eq!(
        String::from_utf8(down_missing.stderr).unwrap(),
        "hint: names given to team down are full session names (with any team prefix); see a2amx list\n"
    );

    // `team down` using team file with a missing session prints `no session` and no hint.
    let missing_file = dir.path().join("config/missing-member.toml");
    std::fs::write(
        &missing_file,
        "team = \"demo\"\n[[session]]\nname = \"ghost\"\ncommand = [\"cat\"]\n",
    )
    .unwrap();
    let down_file = run_binary(
        dir.path(),
        &["team", "down", "--file", missing_file.to_str().unwrap()],
    )
    .unwrap();
    assert!(down_file.status.success());
    assert_eq!(
        String::from_utf8(down_file.stdout).unwrap(),
        "no session demo-ghost\n"
    );
    assert_eq!(String::from_utf8(down_file.stderr).unwrap(), "");

    // P13 abort: first session starts, second fails create_session (`failed NAME: ...`), exits non-zero.
    let abort_file = dir.path().join("config/abort.toml");
    std::fs::write(
        &abort_file,
        "team = \"abort\"\n[[session]]\nname = \"one\"\ncommand = [\"cat\"]\n[[session]]\nname = \"two\"\ncommand = [\"/definitely/missing/a2amx-cmd\"]\n",
    )
    .unwrap();
    let aborted = run_binary(
        dir.path(),
        &[
            "team",
            "up",
            "--detach",
            "--file",
            abort_file.to_str().unwrap(),
        ],
    )
    .unwrap();
    assert!(!aborted.status.success());
    assert_eq!(
        String::from_utf8(aborted.stdout).unwrap(),
        "started abort-one s2\n"
    );
    let abort_err = String::from_utf8(aborted.stderr).unwrap();
    assert_eq!(
        abort_err,
        "a2amx: failed abort-two: Failed to spawn command '/definitely/missing/a2amx-cmd': No such file or directory (os error 2)\n"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn team_up_notes_lost_messages_before_conflict_lines() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon_in(dir.path(), Some("host-a"), Default::default()).await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_recipient_id, _recipient_token, _recipient_addr) = common::new_agent(
        &mut admin,
        dir.path(),
        Some("wanted-wanted"),
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
            to: "wanted-wanted@host-a".into(),
            subject: "restart".into(),
            message: "body".into(),
        })
        .await
        .unwrap();
    assert!(matches!(accepted, Response::Accepted { .. }));
    drop(daemon);

    let _restarted = common::start_daemon_in(dir.path(), Some("host-a"), Default::default()).await;
    let mut admin2 = Client::connect(dir.path()).await.unwrap();
    let (conflict_id, _, _) = common::new_agent(
        &mut admin2,
        dir.path(),
        Some("wanted-wanted"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    std::fs::write(
        dir.path().join("wanted.toml"),
        "team = \"wanted\"\n[[session]]\nname = 'wanted'\ncommand = ['cat']",
    )
    .unwrap();
    let up = run_binary(
        dir.path(),
        &["team", "up", "--detach", "--file", "wanted.toml"],
    )
    .unwrap();
    assert!(!up.status.success());
    assert_eq!(
        String::from_utf8(up.stderr).unwrap(),
        format!(
            "note: 1 message(s) to this team were lost in a daemon restart; see a2amx messages --state undeliverable\nsession wanted-wanted ({conflict_id}) is running with a different team setting; run a2amx team down first\na2amx: team has session conflicts\n"
        )
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn team_up_non_terminal_stdin_skips_attach() {
    let (dir, _daemon) = common::start_daemon().await;
    let file = dir.path().join("noatt.toml");
    std::fs::write(
        &file,
        "team = \"noatt\"\n[[session]]\nname = \"worker\"\ncommand = [\"cat\"]\nattach = true\n",
    )
    .unwrap();
    let up = run_binary(
        dir.path(),
        &["team", "up", "--file", file.to_str().unwrap()],
    )
    .unwrap();
    assert!(up.status.success());
    assert_eq!(
        String::from_utf8(up.stdout).unwrap(),
        "started noatt-worker s1\n"
    );
    assert_eq!(String::from_utf8(up.stderr).unwrap(), "");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn team_down_duplicate_name_reports_killed_then_failed_and_exits_one() {
    let (dir, _daemon) = common::start_daemon().await;
    let file = dir.path().join("dup.toml");
    std::fs::write(
        &file,
        "team = \"t\"\n[[session]]\nname = \"alpha\"\ncommand = [\"cat\"]\n",
    )
    .unwrap();
    assert!(
        run_binary(
            dir.path(),
            &["team", "up", "--detach", "--file", file.to_str().unwrap()],
        )
        .unwrap()
        .status
        .success()
    );
    let down = run_binary(dir.path(), &["team", "down", "t-alpha", "t-alpha"]).unwrap();
    assert!(!down.status.success());
    assert_eq!(
        String::from_utf8(down.stdout).unwrap(),
        "killed t-alpha s1\n"
    );
    assert_eq!(
        String::from_utf8(down.stderr).unwrap(),
        "failed t-alpha: unknown session s1\na2amx: one or more team sessions could not be killed\n"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn team_up_start_validation_precedence() {
    let (dir, _daemon) = common::start_daemon().await;

    // heartbeat error precedes activity error in one session.
    let hb_file = dir.path().join("hb-act.toml");
    std::fs::write(
        &hb_file,
        "team = \"prec\"\n[[session]]\nname = \"other\"\ncommand = [\"cat\"]\n[[session]]\nname = \"one\"\ncommand = [\"cat\"]\nwatch = [\"other\"]\nheartbeat = \"bad\"\n[session.activity]\nworking_after = \"bad\"\n",
    )
    .unwrap();
    let hb_out = run_binary(
        dir.path(),
        &[
            "team",
            "up",
            "--detach",
            "--file",
            hb_file.to_str().unwrap(),
        ],
    )
    .unwrap();
    assert!(!hb_out.status.success());
    assert_eq!(
        String::from_utf8(hb_out.stderr).unwrap(),
        format!(
            "a2amx: team file {}: session prec-one: invalid heartbeat \"bad\": expected a number followed by s, m or h, from 1s to 24h\n",
            hb_file.display()
        )
    );

    // validate_session_confine precedes resolved_confine_request (invalid keep_env + bad confine path).
    let conf_file = dir.path().join("conf.toml");
    std::fs::write(
        &conf_file,
        "team = \"prec\"\n[[session]]\nname = \"two\"\ncommand = [\"cat\"]\n[session.confine]\nenabled = true\nkeep_env = [\"BAD=ENV\"]\nwrite = [\"missing-write-dir\"]\n",
    )
    .unwrap();
    let conf_out = run_binary(
        dir.path(),
        &[
            "team",
            "up",
            "--detach",
            "--file",
            conf_file.to_str().unwrap(),
        ],
    )
    .unwrap();
    assert!(!conf_out.status.success());
    assert_eq!(
        String::from_utf8(conf_out.stderr).unwrap(),
        "a2amx: keep_env \"BAD=ENV\" is not unset by default, so there is nothing to keep\n"
    );
}
