use super::*;

#[test]
fn watching_session_receives_a_peer_exit_message() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());

    let watcher = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "watcher",
            "--harness",
            "generic",
            "--watch",
            "peer",
            "--",
            "sh",
            "-c",
            "sleep 30",
        ],
    );
    assert!(watcher.status.success(), "stderr: {}", stderr(&watcher));
    let peer = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "peer",
            "--harness",
            "generic",
            "--",
            "sh",
            "-c",
            "exit 3",
        ],
    );
    assert!(peer.status.success(), "stderr: {}", stderr(&peer));

    let messages = screen_contains_until(
        &home,
        &["messages", "--session", "watcher"],
        "peer exited: peer (code 3)",
    );
    assert!(messages.status.success(), "stderr: {}", stderr(&messages));
    assert!(
        stdout(&messages).contains("a2amx-daemon@"),
        "{}",
        stdout(&messages)
    );
}

#[test]
fn two_watchers_each_receive_one_peer_exit_message() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());
    for watcher in ["watcher-a", "watcher-b"] {
        let name_args = vec![
            "new".to_owned(),
            "--detach".to_owned(),
            "--name".to_owned(),
            watcher.to_owned(),
            "--harness".to_owned(),
            "generic".to_owned(),
            "--watch".to_owned(),
            "peer".to_owned(),
            "--".to_owned(),
            "sh".to_owned(),
            "-c".to_owned(),
            "sleep 30".to_owned(),
        ];
        let output = run_owned(&home, &name_args);
        assert!(output.status.success(), "stderr: {}", stderr(&output));
    }
    let peer = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "peer",
            "--harness",
            "generic",
            "--",
            "sh",
            "-c",
            "exit 7",
        ],
    );
    assert!(peer.status.success(), "stderr: {}", stderr(&peer));
    for watcher in ["watcher-a", "watcher-b"] {
        let messages = screen_contains_until(
            &home,
            &["messages", "--session", watcher],
            "peer exited: peer (code 7)",
        );
        assert!(messages.status.success(), "stderr: {}", stderr(&messages));
        assert_eq!(
            stdout(&messages)
                .lines()
                .filter(|line| line.contains("peer exited: peer (code 7)"))
                .count(),
            1,
            "{}",
            stdout(&messages)
        );
    }
}

#[test]
fn daemon_ended_peer_does_not_trigger_an_exit_message() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());
    let watcher = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "watcher",
            "--harness",
            "generic",
            "--watch",
            "peer",
            "--",
            "sh",
            "-c",
            "sleep 30",
        ],
    );
    assert!(watcher.status.success(), "stderr: {}", stderr(&watcher));
    let peer = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "peer",
            "--harness",
            "generic",
            "--",
            "sleep",
            "30",
        ],
    );
    assert!(peer.status.success(), "stderr: {}", stderr(&peer));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !stdout(&run(&home, &["list"]))
        .lines()
        .any(|line| line.contains("peer") && line.contains("running"))
    {
        assert!(Instant::now() < deadline, "peer did not become running");
        std::thread::sleep(Duration::from_millis(20));
    }
    let killed = run(&home, &["kill", "peer", "--yes", "--now"]);
    assert!(killed.status.success(), "stderr: {}", stderr(&killed));
    for _ in 0..20 {
        let messages = run(&home, &["messages", "--session", "watcher"]);
        assert!(
            !stdout(&messages).contains("peer exited: peer"),
            "{}",
            stdout(&messages)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn exited_watcher_does_not_receive_a_peer_exit_message() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());
    let watcher = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "watcher",
            "--harness",
            "generic",
            "--watch",
            "peer",
            "--",
            "sh",
            "-c",
            "exit 0",
        ],
    );
    assert!(watcher.status.success(), "stderr: {}", stderr(&watcher));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !stdout(&run(&home, &["list"]))
        .lines()
        .any(|line| line.contains("watcher") && line.contains("exited(0)"))
    {
        assert!(Instant::now() < deadline, "watcher did not exit");
        std::thread::sleep(Duration::from_millis(20));
    }
    let peer = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "peer",
            "--harness",
            "generic",
            "--",
            "sleep",
            "30",
        ],
    );
    assert!(peer.status.success(), "stderr: {}", stderr(&peer));
    for _ in 0..20 {
        let messages = run(&home, &["messages", "--session", "watcher"]);
        assert!(
            !stdout(&messages).contains("peer exited: peer"),
            "{}",
            stdout(&messages)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn team_up_forwards_watch_entries() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    std::fs::create_dir_all(home.join("config")).unwrap();
    let team = home.join("config/team.toml");
    std::fs::write(
        &team,
        r#"
[[session]]
name = "watcher"
command = ["sh", "-c", "sleep 30"]
watch = ["peer"]

[[session]]
name = "peer"
command = ["sh", "-c", "exit 5"]
"#,
    )
    .unwrap();
    assert!(run(&home, &["daemon", "start"]).status.success());
    let file = team.to_string_lossy().into_owned();
    let up = run_owned(
        &home,
        &[
            "team".to_owned(),
            "up".to_owned(),
            "--detach".to_owned(),
            "--file".to_owned(),
            file,
        ],
    );
    assert!(up.status.success(), "stderr: {}", stderr(&up));
    let messages = screen_contains_until(
        &home,
        &["messages", "--session", "watcher"],
        "peer exited: peer (code 5)",
    );
    assert!(messages.status.success(), "stderr: {}", stderr(&messages));
}

#[test]
fn non_watching_session_stays_silent_when_peer_exits() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());
    let silent = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "silent",
            "--harness",
            "generic",
            "--",
            "sleep",
            "30",
        ],
    );
    assert!(silent.status.success(), "stderr: {}", stderr(&silent));
    let peer = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "peer",
            "--harness",
            "generic",
            "--",
            "sh",
            "-c",
            "exit 4",
        ],
    );
    assert!(peer.status.success(), "stderr: {}", stderr(&peer));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !stdout(&run(&home, &["list"]))
        .lines()
        .any(|line| line.contains("peer") && line.contains("exited(4)"))
    {
        assert!(Instant::now() < deadline, "peer did not exit");
        std::thread::sleep(Duration::from_millis(20));
    }
    for _ in 0..20 {
        let messages = run(&home, &["messages", "--session", "silent"]);
        assert!(
            !stdout(&messages).contains("peer exited: peer"),
            "{}",
            stdout(&messages)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn team_down_peer_does_not_trigger_an_exit_message() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());
    let watcher = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "watcher",
            "--harness",
            "generic",
            "--watch",
            "peer",
            "--",
            "sleep",
            "30",
        ],
    );
    assert!(watcher.status.success(), "stderr: {}", stderr(&watcher));
    let peer = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "peer",
            "--harness",
            "generic",
            "--",
            "sleep",
            "30",
        ],
    );
    assert!(peer.status.success(), "stderr: {}", stderr(&peer));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !stdout(&run(&home, &["list"]))
        .lines()
        .any(|line| line.contains("peer") && line.contains("running"))
    {
        assert!(Instant::now() < deadline, "peer did not become running");
        std::thread::sleep(Duration::from_millis(20));
    }
    let down = run(&home, &["team", "down", "--now", "peer"]);
    assert!(down.status.success(), "stderr: {}", stderr(&down));
    for _ in 0..20 {
        let messages = run(&home, &["messages", "--session", "watcher"]);
        assert!(
            !stdout(&messages).contains("peer exited: peer"),
            "{}",
            stdout(&messages)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}
