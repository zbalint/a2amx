use super::*;

fn screen_until(home: &Path, args: &[&str], expected: &str) -> Output {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let output = run(home, args);
        if !output.status.success() || stdout(&output) == expected || Instant::now() >= deadline {
            return output;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// An executable named `claude` that only sleeps, so no real harness starts.
fn fake_claude(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("claude");
    std::fs::write(&path, "#!/bin/sh\nexec sleep 30\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[test]
fn harness_is_inferred_from_the_command() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");

    let _cleanup = Cleanup(&home);
    let claude = fake_claude(dir.path());
    assert!(run(&home, &["daemon", "start"]).status.success());

    let created = run(&home, &["new", "--detach", "--", claude.to_str().unwrap()]);

    assert!(created.status.success(), "stderr: {}", stderr(&created));
    assert!(stdout(&run(&home, &["list", "--details"])).contains("--mcp-config"));
}
#[test]
fn explicit_generic_harness_wins_over_inference() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    let claude = fake_claude(dir.path());
    assert!(run(&home, &["daemon", "start"]).status.success());

    let created = run(
        &home,
        &[
            "new",
            "--detach",
            "--harness",
            "generic",
            "--",
            claude.to_str().unwrap(),
        ],
    );

    assert!(created.status.success(), "stderr: {}", stderr(&created));
    assert!(!stdout(&run(&home, &["list", "--details"])).contains("--mcp-config"));
}

#[test]
fn screen_prints_trimmed_visible_rows() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());
    let created = run(
        &home,
        &[
            "new",
            "--detach",
            "--harness",
            "generic",
            "--",
            "sh",
            "-c",
            "printf 'first  \\n\\nthird   \\n\\n'; sleep 30",
        ],
    );
    assert!(created.status.success(), "stderr: {}", stderr(&created));

    let output = screen_until(&home, &["screen", "s1"], "first\n\nthird\n");
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output), "first\n\nthird\n");
}

#[test]
fn screen_resolves_session_name() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
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
            "sh",
            "-c",
            "printf 'named\\n'; sleep 30",
        ],
    );
    assert!(created.status.success(), "stderr: {}", stderr(&created));

    let output = screen_until(&home, &["screen", "worker"], "named\n");
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output), "named\n");
}

#[test]
fn screen_rows_keeps_only_the_last_lines() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());
    let created = run(
        &home,
        &[
            "new",
            "--detach",
            "--harness",
            "generic",
            "--",
            "sh",
            "-c",
            "printf 'one\\ntwo\\nthree\\n'; sleep 30",
        ],
    );
    assert!(created.status.success(), "stderr: {}", stderr(&created));

    let output = screen_until(&home, &["screen", "s1", "--rows", "2"], "two\nthree\n");
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output), "two\nthree\n");
}

#[test]
fn screen_reads_the_active_alternate_screen() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());
    let created = run(
        &home,
        &[
            "new",
            "--detach",
            "--harness",
            "generic",
            "--",
            "sh",
            "-c",
            "printf '\\033[?1049hALT\\n'; sleep 30",
        ],
    );
    assert!(created.status.success(), "stderr: {}", stderr(&created));

    let output = screen_until(&home, &["screen", "s1"], "ALT\n");
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output), "ALT\n");
}

#[test]
fn screen_does_not_resize_or_detach_an_attached_session() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());
    let created = run(
        &home,
        &[
            "new",
            "--detach",
            "--harness",
            "generic",
            "--",
            "sh",
            "-c",
            "printf 'attached\\n'; sleep 30",
        ],
    );
    assert!(created.status.success(), "stderr: {}", stderr(&created));

    let attach_args = vec!["attach".to_owned(), "s1".to_owned()];
    let mut attached = common::pty::PtyHarness::spawn(&attach_args, &home).unwrap();
    attached
        .wait_for_text("attached", Duration::from_secs(10))
        .unwrap();
    let before = stdout(&run(&home, &["list"]));
    let before_cells = common::list_cells(&before, "s1");
    let before_size = before_cells[12].to_owned();

    let output = screen_until(&home, &["screen", "s1"], "attached\n");
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output), "attached\n");

    let after = stdout(&run(&home, &["list"]));
    let after_cells = common::list_cells(&after, "s1");
    assert_eq!(after_cells[8], "yes");
    assert_eq!(after_cells[12], before_size);
}

#[test]
fn screen_unknown_session_matches_kill_error() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());

    let kill = run(&home, &["kill", "missing"]);
    let screen = run(&home, &["screen", "missing"]);
    assert!(!kill.status.success());
    assert!(!screen.status.success());
    assert_eq!(stderr(&screen), stderr(&kill));
}

#[test]
fn screen_reads_the_last_screen_after_exit() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());
    let created = run(
        &home,
        &[
            "new",
            "--detach",
            "--harness",
            "generic",
            "--",
            "sh",
            "-c",
            "printf 'exited\\n'; exit 7",
        ],
    );
    assert!(created.status.success(), "stderr: {}", stderr(&created));

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let listed = stdout(&run(&home, &["list"]));
        if listed
            .lines()
            .find(|line| line.starts_with("s1 "))
            .is_some_and(|line| {
                line.split_whitespace()
                    .nth(4)
                    .is_some_and(|state| state.starts_with("exited("))
            })
        {
            break;
        }
        assert!(Instant::now() < deadline, "session did not exit: {listed}");
        std::thread::sleep(Duration::from_millis(20));
    }

    let output = screen_until(&home, &["screen", "s1"], "exited\n");
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output), "exited\n");
}

#[test]
fn screen_wide_character_is_not_duplicated() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());
    let created = run(
        &home,
        &[
            "new",
            "--detach",
            "--harness",
            "generic",
            "--",
            "sh",
            "-c",
            "printf 'A界B\\n'; sleep 30",
        ],
    );
    assert!(created.status.success(), "stderr: {}", stderr(&created));

    let output = screen_until(&home, &["screen", "s1"], "A界B\n");
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output), "A界B\n");
}
