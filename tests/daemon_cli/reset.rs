use super::*;

fn session_diagnostics(home: &Path, session: &str) -> String {
    let explain = run(home, &["explain", session]);
    let screen = run(home, &["screen", session]);
    format!(
        "explain stdout:\n{}\nexplain stderr:\n{}\nscreen stdout:\n{}\nscreen stderr:\n{}",
        stdout(&explain),
        stderr(&explain),
        stdout(&screen),
        stderr(&screen)
    )
}

fn fake_reset_composer(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("fake-reset-composer.py");
    std::fs::write(
        &path,
        r#"import sys
sys.stdout.write("\x1b[?2004h")
sys.stdout.flush()
for raw in sys.stdin.buffer:
    line = raw.decode(errors="replace").replace("\x1b[200~", "").replace("\x1b[201~", "").rstrip("\r\n")
    if line == "/clear":
        print("marker:/clear", flush=True)
    else:
        print("got:" + line, flush=True)
"#,
    )
    .expect("write fake reset composer");
    path
}

fn fake_timeout_composer(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("fake-timeout-composer.py");
    std::fs::write(
        &path,
        r#"import sys
sys.stdout.write("\x1b[?2004h")
sys.stdout.flush()
for raw in sys.stdin.buffer:
    line = raw.decode(errors="replace").replace("\x1b[200~", "").replace("\x1b[201~", "").rstrip("\r\n")
    if line == "/clear":
        print("marker:/clear", flush=True)
    elif line == "/second":
        print("got:/second", flush=True)
        sys.stdout.write("\x1b[?2004l")
        sys.stdout.flush()
"#,
    )
    .expect("write fake timeout composer");
    path
}

#[test]
fn reset_uses_default_clear_sequence() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    let fixture = fake_reset_composer(dir.path());
    assert!(run(&home, &["daemon", "start"]).status.success());
    let created = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "worker",
            "--harness",
            "generic",
            "--",
            "python3",
            "-u",
            fixture.to_str().unwrap(),
        ],
    );
    assert!(created.status.success(), "stderr: {}", stderr(&created));
    let reset = run(&home, &["reset", "worker"]);
    assert!(
        reset.status.success(),
        "stdout: {}\nstderr: {}",
        stdout(&reset),
        stderr(&reset)
    );
    assert_eq!(stdout(&reset), "reset s1: 1 step\n");
    let screen = screen_contains_until(&home, &["screen", "worker"], "marker:/clear");
    assert!(screen.status.success(), "stderr: {}", stderr(&screen));
    assert!(stdout(&screen).contains("marker:/clear"));
}

#[test]
fn reset_rejects_a_draft_without_typing() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    let fixture = fake_reset_composer(dir.path());
    assert!(run(&home, &["daemon", "start"]).status.success());
    let created = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "worker",
            "--harness",
            "generic",
            "--",
            "python3",
            "-u",
            fixture.to_str().unwrap(),
        ],
    );
    assert!(created.status.success(), "stderr: {}", stderr(&created));
    let args = vec![
        "--home".to_owned(),
        home.to_string_lossy().into_owned(),
        "attach".to_owned(),
        "worker".to_owned(),
    ];
    let mut terminal = PtyHarness::spawn(&args, &home).unwrap();
    terminal
        .wait_for_text("worker", Duration::from_secs(10))
        .unwrap();
    terminal.send(b"draft").unwrap();
    std::thread::sleep(Duration::from_millis(200));
    let reset = run(&home, &["reset", "worker"]);
    assert!(!reset.status.success());
    assert!(
        stderr(&reset).contains("draft_present"),
        "{}",
        stderr(&reset)
    );
    drop(terminal);
}

#[test]
fn reset_reports_not_ready_exited_and_unknown_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());

    let not_ready = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "not-ready",
            "--harness",
            "generic",
            "--",
            "sleep",
            "30",
        ],
    );
    assert!(not_ready.status.success(), "stderr: {}", stderr(&not_ready));
    let reset = run(&home, &["reset", "not-ready"]);
    assert!(!reset.status.success());
    assert!(stderr(&reset).contains("not_ready"), "{}", stderr(&reset));
    assert!(stderr(&reset).contains("step 1 of 1"), "{}", stderr(&reset));

    let exited = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "exited",
            "--harness",
            "generic",
            "--",
            "true",
        ],
    );
    assert!(exited.status.success(), "stderr: {}", stderr(&exited));
    let reset = run(&home, &["reset", "exited"]);
    assert!(!reset.status.success());
    assert!(stderr(&reset).contains("exited"), "{}", stderr(&reset));

    let reset = run(&home, &["reset", "missing"]);
    assert!(!reset.status.success());
    assert!(
        stderr(&reset).contains("unknown_session"),
        "{}",
        stderr(&reset)
    );
}

#[test]
fn reset_times_out_when_second_step_stays_not_ready() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    let fixture = fake_timeout_composer(dir.path());
    assert!(run(&home, &["daemon", "start"]).status.success());
    let created = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "worker",
            "--harness",
            "generic",
            "--reset",
            "/clear",
            "--reset",
            "/second",
            "--",
            "python3",
            "-u",
            fixture.to_str().unwrap(),
        ],
    );
    assert!(created.status.success(), "stderr: {}", stderr(&created));
    let reset = run(&home, &["reset", "worker"]);
    assert!(
        !reset.status.success(),
        "stdout: {}\n{}",
        stdout(&reset),
        session_diagnostics(&home, "worker")
    );
    let message = stderr(&reset);
    assert!(
        message.contains("step_timeout"),
        "{message}\n{}",
        session_diagnostics(&home, "worker")
    );
    assert!(
        message.contains("step 2 of 2"),
        "{message}\n{}",
        session_diagnostics(&home, "worker")
    );
    let screen = screen_contains_until(&home, &["screen", "worker"], "marker:/clear");
    assert!(
        screen.status.success(),
        "stderr: {}\n{}",
        stderr(&screen),
        session_diagnostics(&home, "worker")
    );
    let text = stdout(&screen);
    assert!(
        text.contains("marker:/clear"),
        "{text:?}\n{}",
        session_diagnostics(&home, "worker")
    );
    assert!(
        text.contains("got:/second"),
        "{text:?}\n{}",
        session_diagnostics(&home, "worker")
    );
    let retry = run(&home, &["reset", "worker"]);
    assert!(
        !retry.status.success(),
        "{}",
        session_diagnostics(&home, "worker")
    );
    assert!(
        !stderr(&retry).contains("busy"),
        "{}\n{}",
        stderr(&retry),
        session_diagnostics(&home, "worker")
    );
    assert!(
        stderr(&retry).contains("not_ready"),
        "{}\n{}",
        stderr(&retry),
        session_diagnostics(&home, "worker")
    );
}

#[test]
fn reset_runs_configured_steps_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    let fixture = fake_reset_composer(dir.path());
    assert!(run(&home, &["daemon", "start"]).status.success());
    let created = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "worker",
            "--harness",
            "generic",
            "--reset",
            "/clear",
            "--reset",
            "/prewalk restart",
            "--",
            "python3",
            "-u",
            fixture.to_str().unwrap(),
        ],
    );
    assert!(created.status.success(), "stderr: {}", stderr(&created));
    let reset = run(&home, &["reset", "worker"]);
    assert!(
        reset.status.success(),
        "stdout: {}\nstderr: {}\n{}",
        stdout(&reset),
        stderr(&reset),
        session_diagnostics(&home, "worker")
    );
    assert_eq!(
        stdout(&reset),
        "reset s1: 2 steps\n",
        "{}",
        session_diagnostics(&home, "worker")
    );
    let screen = screen_contains_until(&home, &["screen", "worker"], "got:/prewalk restart");
    assert!(
        screen.status.success(),
        "stderr: {}\n{}",
        stderr(&screen),
        session_diagnostics(&home, "worker")
    );
    let text = stdout(&screen);
    assert!(
        text.contains("marker:/clear"),
        "{text:?}\n{}",
        session_diagnostics(&home, "worker")
    );
    assert!(
        text.contains("got:/prewalk restart"),
        "{text:?}\n{}",
        session_diagnostics(&home, "worker")
    );
    assert!(
        text.find("marker:/clear") < text.find("got:/prewalk restart"),
        "{text:?}\n{}",
        session_diagnostics(&home, "worker")
    );
}

#[test]
fn team_reset_file_resets_sessions_in_file_order() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    let fixture = fake_reset_composer(dir.path());
    std::fs::create_dir_all(home.join("config")).unwrap();
    let file = home.join("config/team.toml");
    std::fs::write(
        &file,
        format!(
            "[[session]]\nname = \"alpha\"\ncommand = [\"python3\", \"-u\", \"{}\"]\n\
             [[session]]\nname = \"beta\"\ncommand = [\"python3\", \"-u\", \"{}\"]\n",
            fixture.display(),
            fixture.display()
        ),
    )
    .unwrap();
    assert!(run(&home, &["daemon", "start"]).status.success());
    let file_arg = file.to_str().unwrap();
    let started = run(&home, &["team", "up", "--detach", "--file", file_arg]);
    assert!(started.status.success(), "stderr: {}", stderr(&started));

    let reset = run(&home, &["team", "reset", "--file", file_arg, "--yes"]);
    assert!(
        reset.status.success(),
        "stdout: {}\nstderr: {}",
        stdout(&reset),
        stderr(&reset)
    );
    assert_eq!(
        stdout(&reset),
        "reset config-alpha s1: 1 step\nreset config-beta s2: 1 step\n"
    );
    for name in ["config-alpha", "config-beta"] {
        let screen = screen_contains_until(&home, &["screen", name], "marker:/clear");
        assert!(screen.status.success(), "stderr: {}", stderr(&screen));
    }
}

#[test]
fn team_reset_reports_missing_and_exited_before_resetting_ready() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    let fixture = fake_reset_composer(dir.path());
    std::fs::create_dir_all(home.join("config")).unwrap();
    let file = home.join("config/team.toml");
    std::fs::write(
        &file,
        format!(
            "team = \"config\"\n[[session]]\nname = \"missing\"\ncommand = [\"{}\"]\n\
             [[session]]\nname = \"exited\"\ncommand = [\"true\"]\n\
             [[session]]\nname = \"ready\"\ncommand = [\"{}\"]\n",
            fixture.display(),
            fixture.display()
        ),
    )
    .unwrap();
    let start_file = home.join("config/start.toml");
    std::fs::write(
        &start_file,
        format!(
            "team = \"config\"\n[[session]]\nname = \"exited\"\ncommand = [\"true\"]\n\
             [[session]]\nname = \"ready\"\ncommand = [\"python3\", \"-u\", \"{}\"]\n",
            fixture.display()
        ),
    )
    .unwrap();
    assert!(run(&home, &["daemon", "start"]).status.success());
    let start_file_arg = start_file.to_str().unwrap();
    let started = run(&home, &["team", "up", "--detach", "--file", start_file_arg]);
    assert!(started.status.success(), "stderr: {}", stderr(&started));
    let file_arg = file.to_str().unwrap();
    let reset = run(&home, &["team", "reset", "--file", file_arg, "--yes"]);

    assert!(
        reset.status.success(),
        "stdout: {}\nstderr: {}",
        stdout(&reset),
        stderr(&reset)
    );
    assert_eq!(
        stdout(&reset),
        "no session config-missing\nskipped config-exited s1 (exited)\nreset config-ready s2: 1 step\n"
    );
}

#[test]
fn team_reset_continues_after_a_not_ready_session_and_fails() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    let fixture = fake_reset_composer(dir.path());
    std::fs::create_dir_all(home.join("config")).unwrap();
    let file = home.join("config/team.toml");
    std::fs::write(
        &file,
        format!(
            "[[session]]\nname = \"first\"\ncommand = [\"python3\", \"-u\", \"{}\"]\n\
             [[session]]\nname = \"not-ready\"\ncommand = [\"sleep\", \"30\"]\n\
             [[session]]\nname = \"last\"\ncommand = [\"python3\", \"-u\", \"{}\"]\n",
            fixture.display(),
            fixture.display()
        ),
    )
    .unwrap();
    assert!(run(&home, &["daemon", "start"]).status.success());
    let file_arg = file.to_str().unwrap();
    let started = run(&home, &["team", "up", "--detach", "--file", file_arg]);
    assert!(started.status.success(), "stderr: {}", stderr(&started));
    let reset = run(&home, &["team", "reset", "--file", file_arg, "--yes"]);

    assert!(!reset.status.success(), "stdout: {}", stdout(&reset));
    assert_eq!(
        stderr(&reset),
        "failed config-not-ready: not_ready: reset gate rejected step 1 of 1: screen not ready\na2amx: one or more team sessions could not be reset\n"
    );
    assert_eq!(
        stdout(&reset),
        "reset config-first s1: 1 step\nreset config-last s3: 1 step\n"
    );
}

#[test]
fn team_reset_except_leaves_the_named_session_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    let fixture = fake_reset_composer(dir.path());
    std::fs::create_dir_all(home.join("config")).unwrap();
    let file = home.join("config/team.toml");
    std::fs::write(
        &file,
        format!(
            "[[session]]\nname = \"keep\"\ncommand = [\"python3\", \"-u\", \"{}\"]\n\
             [[session]]\nname = \"reset\"\ncommand = [\"python3\", \"-u\", \"{}\"]\n",
            fixture.display(),
            fixture.display()
        ),
    )
    .unwrap();
    assert!(run(&home, &["daemon", "start"]).status.success());
    let file_arg = file.to_str().unwrap();
    let started = run(&home, &["team", "up", "--detach", "--file", file_arg]);
    assert!(started.status.success(), "stderr: {}", stderr(&started));
    let reset = run(
        &home,
        &[
            "team",
            "reset",
            "--file",
            file_arg,
            "--except",
            "config-keep",
            "--except",
            "not-selected",
            "--yes",
        ],
    );

    assert!(reset.status.success(), "stderr: {}", stderr(&reset));
    assert_eq!(stdout(&reset), "reset config-reset s2: 1 step\n");
    let keep_screen = run(&home, &["screen", "config-keep"]);
    assert!(
        keep_screen.status.success(),
        "stderr: {}",
        stderr(&keep_screen)
    );
    assert!(!stdout(&keep_screen).contains("marker:/clear"));
}

#[test]
fn team_reset_requires_yes_for_nonterminal_invocation() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    let fixture = fake_reset_composer(dir.path());
    std::fs::create_dir_all(home.join("config")).unwrap();
    let file = home.join("config/team.toml");
    std::fs::write(
        &file,
        format!(
            "[[session]]\nname = \"alpha\"\ncommand = [\"python3\", \"-u\", \"{}\"]\n\
             [[session]]\nname = \"beta\"\ncommand = [\"python3\", \"-u\", \"{}\"]\n",
            fixture.display(),
            fixture.display()
        ),
    )
    .unwrap();
    assert!(run(&home, &["daemon", "start"]).status.success());
    let file_arg = file.to_str().unwrap();
    let started = run(&home, &["team", "up", "--detach", "--file", file_arg]);
    assert!(started.status.success(), "stderr: {}", stderr(&started));
    let reset = run(&home, &["team", "reset", "--file", file_arg]);

    assert!(!reset.status.success());
    assert_eq!(stdout(&reset), "config-alpha s1\nconfig-beta s2\n");
    assert_eq!(
        stderr(&reset),
        "a2amx: refusing to reset: 2 running session(s); pass --yes to reset them\n"
    );
    for name in ["config-alpha", "config-beta"] {
        let screen = run(&home, &["screen", name]);
        assert!(screen.status.success(), "stderr: {}", stderr(&screen));
        assert!(!stdout(&screen).contains("marker:/clear"));
    }
}

#[test]
fn team_reset_accepts_explicit_names_and_rejects_file_name_mix() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    let fixture = fake_reset_composer(dir.path());
    assert!(run(&home, &["daemon", "start"]).status.success());
    for name in ["alpha", "beta"] {
        let created = run(
            &home,
            &[
                "new",
                "--detach",
                "--name",
                name,
                "--harness",
                "generic",
                "--",
                "python3",
                "-u",
                fixture.to_str().unwrap(),
            ],
        );
        assert!(created.status.success(), "stderr: {}", stderr(&created));
    }

    let reset = run(&home, &["team", "reset", "alpha", "beta", "--yes"]);
    assert!(
        reset.status.success(),
        "stdout: {}\nstderr: {}",
        stdout(&reset),
        stderr(&reset)
    );
    assert_eq!(
        stdout(&reset),
        "reset alpha s1: 1 step\nreset beta s2: 1 step\n"
    );

    std::fs::create_dir_all(home.join("config")).unwrap();
    let file = home.join("config/team.toml");
    std::fs::write(
        &file,
        "[[session]]\nname = \"alpha\"\ncommand = [\"cat\"]\n",
    )
    .unwrap();
    let mixed = run(
        &home,
        &[
            "team",
            "reset",
            "--file",
            file.to_str().unwrap(),
            "alpha",
            "--yes",
        ],
    );
    assert!(!mixed.status.success());
    assert!(
        stderr(&mixed).contains("cannot be used with"),
        "{}",
        stderr(&mixed)
    );
}

#[test]
fn new_rejects_invalid_reset_configuration() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());

    let cases = [
        (
            vec![
                "new".to_owned(),
                "--detach".to_owned(),
                "--reset".to_owned(),
                "clear".to_owned(),
                "--".to_owned(),
                "true".to_owned(),
            ],
            "must start with '/'",
        ),
        (
            vec![
                "new".to_owned(),
                "--detach".to_owned(),
                "--reset".to_owned(),
                "/bad\ncommand".to_owned(),
                "--".to_owned(),
                "true".to_owned(),
            ],
            "control characters",
        ),
        (
            vec![
                "new".to_owned(),
                "--detach".to_owned(),
                "--reset".to_owned(),
                format!("/{}", "x".repeat(200)),
                "--".to_owned(),
                "true".to_owned(),
            ],
            "at most 200 bytes",
        ),
        (
            vec![
                "new".to_owned(),
                "--detach".to_owned(),
                "--control-from".to_owned(),
                "Bad_Name".to_owned(),
                "--".to_owned(),
                "true".to_owned(),
            ],
            "invalid name",
        ),
    ];
    for (args, expected) in cases {
        let output = run_owned(&home, &args);
        assert!(!output.status.success(), "{args:?}");
        assert!(
            stderr(&output).contains(expected),
            "{expected}: {}",
            stderr(&output)
        );
    }

    let mut too_many = vec!["new".to_owned(), "--detach".to_owned()];
    for _ in 0..9 {
        too_many.push("--reset".to_owned());
        too_many.push("/clear".to_owned());
    }
    too_many.extend(["--".to_owned(), "true".to_owned()]);
    let output = run_owned(&home, &too_many);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("at most 8 steps"));
}

#[test]
fn reset_rejects_a_concurrent_request_as_busy() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    let fixture = fake_reset_composer(dir.path());
    assert!(run(&home, &["daemon", "start"]).status.success());
    let created = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "worker",
            "--harness",
            "generic",
            "--reset",
            "/clear",
            "--reset",
            "/second",
            "--",
            "python3",
            "-u",
            fixture.to_str().unwrap(),
        ],
    );
    assert!(created.status.success(), "stderr: {}", stderr(&created));
    let first = a2amx(&home, &["reset", "worker"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn first reset");
    std::thread::sleep(Duration::from_millis(200));
    let second = run(&home, &["reset", "worker"]);
    assert!(
        !second.status.success(),
        "second reset unexpectedly succeeded"
    );
    assert!(stderr(&second).contains("busy"), "{}", stderr(&second));
    let first = first.wait_with_output().expect("wait first reset");
    assert!(first.status.success(), "stderr: {}", stderr(&first));
    assert_eq!(stdout(&first), "reset s1: 2 steps\n");
}

#[test]
fn team_reset_characterizes_confirm_no_and_attached_preview() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    let fixture = fake_reset_composer(dir.path());
    std::fs::create_dir_all(home.join("config")).unwrap();
    let file = home.join("config/team.toml");
    std::fs::write(
        &file,
        format!(
            "team = \"cfg\"\n[[session]]\nname = \"alpha\"\ncommand = [\"python3\", \"-u\", \"{}\"]\n",
            fixture.display()
        ),
    )
    .unwrap();
    assert!(run(&home, &["daemon", "start"]).status.success());
    let file_arg = file.to_str().unwrap();
    assert!(
        run(&home, &["team", "up", "--detach", "--file", file_arg])
            .status
            .success()
    );

    // Attach to cfg-alpha in a PTY so it is marked attached.
    let mut attached_pty = PtyHarness::spawn(
        &[
            "--home".to_string(),
            home.to_str().unwrap().to_string(),
            "attach".to_string(),
            "cfg-alpha".to_string(),
        ],
        dir.path(),
    )
    .unwrap();
    let wait = Duration::from_secs(5);
    attached_pty.wait_for_text("[s1]", wait).ok();

    // Without --include-attached, cfg-alpha is skipped with exact line.
    let skipped = run(&home, &["team", "reset", "--file", file_arg, "--yes"]);
    assert!(skipped.status.success());
    assert_eq!(
        stdout(&skipped),
        "skipped cfg-alpha s1 (attached; use --include-attached or name it)\n"
    );

    // Non-terminal with --include-attached prints preview with ` (attached)` before refusing.
    let nonterm = run(
        &home,
        &["team", "reset", "--file", file_arg, "--include-attached"],
    );
    assert!(!nonterm.status.success());
    assert_eq!(stdout(&nonterm), "cfg-alpha s1 (attached)\n");
    assert_eq!(
        stderr(&nonterm),
        "a2amx: refusing to reset: 1 running session(s); pass --yes to reset them\n"
    );

    // Terminal "n" prints `not reset` and exits 0.
    let mut pty = PtyHarness::spawn(
        &[
            "--home".to_string(),
            home.to_str().unwrap().to_string(),
            "team".to_string(),
            "reset".to_string(),
            "--file".to_string(),
            file_arg.to_string(),
            "--include-attached".to_string(),
        ],
        dir.path(),
    )
    .unwrap();
    pty.wait_for_text(
        "Reset 1 session(s)? This clears their conversations. [y/N]",
        wait,
    )
    .unwrap();
    pty.send(b"n\r").unwrap();
    pty.wait_for_text("not reset", wait).unwrap();
    assert_eq!(pty.wait_exit(wait).unwrap(), 0);

    attached_pty.send(&[2, b'd']).unwrap();
    let _ = attached_pty.wait_exit(wait);
}

#[test]
fn team_reset_exited_only_skips_confirmation_without_yes() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    std::fs::create_dir_all(home.join("config")).unwrap();
    let file = home.join("config/team.toml");
    std::fs::write(
        &file,
        "[[session]]\nname = \"alpha\"\ncommand = [\"true\"]\n",
    )
    .unwrap();
    assert!(run(&home, &["daemon", "start"]).status.success());
    let file_arg = file.to_str().unwrap();
    let started = run(&home, &["team", "up", "--detach", "--file", file_arg]);
    assert!(started.status.success(), "stderr: {}", stderr(&started));

    let mut waited = 0;
    loop {
        let status = run(&home, &["team", "status", "--file", file_arg]);
        if stdout(&status).contains("exited") {
            break;
        }
        assert!(waited < 50);
        waited += 1;
        std::thread::sleep(Duration::from_millis(50));
    }

    let reset = run(&home, &["team", "reset", "--file", file_arg]);
    assert!(reset.status.success(), "stderr: {}", stderr(&reset));
    assert_eq!(stdout(&reset), "skipped config-alpha s1 (exited)\n");
    assert_eq!(stderr(&reset), "");
}
