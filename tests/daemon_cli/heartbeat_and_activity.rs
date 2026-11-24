use super::*;

#[test]
fn idle_watcher_receives_heartbeat_for_busy_peer() {
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
            "arch",
            "--harness",
            "generic",
            "--watch",
            "dev",
            "--heartbeat",
            "2s",
            "--",
            "sh",
            "-c",
            "printf '\\033[?2004h'; sleep 60",
        ],
    );
    assert!(watcher.status.success(), "stderr: {}", stderr(&watcher));
    let peer = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "dev",
            "--harness",
            "generic",
            "--",
            "sh",
            "-c",
            "printf '\\033[?2004h'; while :; do printf 'peer output\\n'; sleep 1; done",
        ],
    );
    assert!(peer.status.success(), "stderr: {}", stderr(&peer));
    let messages = screen_contains_until(&home, &["messages", "--session", "arch"], "heartbeat");
    assert!(messages.status.success(), "stderr: {}", stderr(&messages));
    assert!(
        stdout(&messages).contains("a2amx-daemon@"),
        "{}",
        stdout(&messages)
    );
}

#[test]
fn idle_watcher_skips_heartbeat_for_ready_peer() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());
    let peer = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "dev",
            "--harness",
            "generic",
            "--",
            "sh",
            "-c",
            "printf '\\033[?2004h'; sleep 60",
        ],
    );
    assert!(peer.status.success(), "stderr: {}", stderr(&peer));
    std::thread::sleep(Duration::from_millis(10_500));
    let watcher = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "arch",
            "--harness",
            "generic",
            "--watch",
            "dev",
            "--heartbeat",
            "2s",
            "--",
            "sh",
            "-c",
            "printf '\\033[?2004h'; sleep 60",
        ],
    );
    assert!(watcher.status.success(), "stderr: {}", stderr(&watcher));
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        let messages = run(&home, &["messages", "--session", "arch"]);
        assert!(
            !stdout(&messages).contains("heartbeat"),
            "{}",
            stdout(&messages)
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[tokio::test]
async fn idle_watcher_receives_heartbeat_for_queued_idle_peer() {
    use a2amx::client::Client;
    use a2amx::harness::{Deliver, Harness};
    use a2amx::wire::{Request, Response};

    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());
    let peer = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "dev",
            "--harness",
            "generic",
            "--deliver",
            "hold",
            "--",
            "sh",
            "-c",
            "printf '\\033[?2004h'; sleep 60",
        ],
    );
    assert!(peer.status.success(), "stderr: {}", stderr(&peer));
    tokio::time::sleep(Duration::from_millis(10_500)).await;
    let mut admin = Client::connect(&home).await.unwrap();
    let (_, token, address) = common::new_agent(
        &mut admin,
        &home,
        Some("sender"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let mut sender = Client::connect_addr(address, &token).await.unwrap();
    assert!(matches!(
        sender
            .request(Request::SendMessage {
                to: "dev".into(),
                subject: "Queued work".into(),
                message: "Review this when ready.".into(),
            })
            .await
            .unwrap(),
        Response::Accepted { .. }
    ));
    let queued = run(
        &home,
        &["messages", "--session", "dev", "--state", "pending"],
    );
    assert!(queued.status.success(), "stderr: {}", stderr(&queued));
    assert!(
        stdout(&queued).contains("Queued work"),
        "{}",
        stdout(&queued)
    );
    let watcher = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "arch",
            "--harness",
            "generic",
            "--watch",
            "dev",
            "--heartbeat",
            "2s",
            "--",
            "sh",
            "-c",
            "printf '\\033[?2004h'; sleep 60",
        ],
    );
    assert!(watcher.status.success(), "stderr: {}", stderr(&watcher));
    let messages = screen_contains_until(&home, &["messages", "--session", "arch"], "heartbeat");
    assert!(messages.status.success(), "stderr: {}", stderr(&messages));
    assert!(
        stdout(&messages).contains("a2amx-daemon@"),
        "{}",
        stdout(&messages)
    );
}

#[test]
fn held_delivery_keeps_at_most_one_heartbeat_open() {
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
            "arch",
            "--harness",
            "generic",
            "--deliver",
            "hold",
            "--watch",
            "dev",
            "--heartbeat",
            "2s",
            "--",
            "sh",
            "-c",
            "printf '\\033[?2004h'; sleep 60",
        ],
    );
    assert!(watcher.status.success(), "stderr: {}", stderr(&watcher));
    let peer = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "dev",
            "--harness",
            "generic",
            "--",
            "sh",
            "-c",
            "sleep 60",
        ],
    );
    assert!(peer.status.success(), "stderr: {}", stderr(&peer));
    let messages = screen_contains_until(&home, &["messages", "--session", "arch"], "heartbeat");
    assert!(messages.status.success(), "stderr: {}", stderr(&messages));
    std::thread::sleep(Duration::from_secs(5));
    let messages = run(&home, &["messages", "--session", "arch"]);
    let count = stdout(&messages)
        .lines()
        .filter(|line| line.contains("heartbeat"))
        .count();
    assert_eq!(count, 1, "{}", stdout(&messages));
}

#[test]
fn activity_notices_report_peer_transitions_once_in_order() {
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
            "w",
            "--harness",
            "generic",
            "--deliver",
            "auto",
            "--watch",
            "p",
            "--watch",
            "a",
            "--activity-working-after",
            "1s",
            "--activity-idle-after",
            "1s",
            "--",
            "sh",
            "-c",
            "stty -echo; printf '\\033[?2004h'; cat",
        ],
    );
    assert!(watcher.status.success(), "stderr: {}", stderr(&watcher));
    let peer = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "p",
            "--harness",
            "generic",
            "--",
            "sh",
            "-c",
            "printf '\\033[?2004h'; sleep 15; while :; do printf x; sleep 1; done",
        ],
    );
    assert!(peer.status.success(), "stderr: {}", stderr(&peer));
    let first_alphabetically = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "a",
            "--harness",
            "generic",
            "--",
            "sh",
            "-c",
            "printf '\\033[?2004h'; sleep 15; while :; do printf x; sleep 1; done",
        ],
    );
    assert!(
        first_alphabetically.status.success(),
        "stderr: {}",
        stderr(&first_alphabetically)
    );
    let third = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "third",
            "--harness",
            "generic",
            "--",
            "sh",
            "-c",
            "printf '\\033[?2004h'; sleep 30",
        ],
    );
    assert!(third.status.success(), "stderr: {}", stderr(&third));
    let idle_a = "- a: idle (was working";
    let idle_p = "- p: idle (was working";
    let working_a = "- a: working (was idle";
    let working_p = "- p: working (was idle";
    let deadline = Instant::now() + Duration::from_secs(30);
    let messages = loop {
        let output = run(&home, &["screen", "w"]);
        let text = stdout(&output);
        let complete_messages = text.split("peer activity:").skip(1).collect::<Vec<_>>();
        if complete_messages
            .iter()
            .any(|message| message.contains(idle_a))
            && complete_messages
                .iter()
                .any(|message| message.contains(idle_p))
            && complete_messages
                .iter()
                .any(|message| message.contains(working_a))
            && complete_messages
                .iter()
                .any(|message| message.contains(working_p))
        {
            break output;
        }
        if Instant::now() >= deadline {
            break output;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(messages.status.success(), "stderr: {}", stderr(&messages));
    let text = stdout(&messages);
    let messages = text.split("peer activity:").skip(1).collect::<Vec<_>>();
    assert_eq!(
        messages
            .iter()
            .map(|message| message.matches(idle_a).count())
            .sum::<usize>(),
        1,
        "{text}"
    );
    assert_eq!(
        messages
            .iter()
            .map(|message| message.matches(idle_p).count())
            .sum::<usize>(),
        1,
        "{text}"
    );
    assert_eq!(
        messages
            .iter()
            .map(|message| message.matches(working_a).count())
            .sum::<usize>(),
        1,
        "{text}"
    );
    assert_eq!(
        messages
            .iter()
            .map(|message| message.matches(working_p).count())
            .sum::<usize>(),
        1,
        "{text}"
    );
    let message_text = messages.join("peer activity:");
    let idle_a_position = message_text.find(idle_a).expect(&text);
    let idle_p_position = message_text.find(idle_p).expect(&text);
    let working_a_position = message_text.find(working_a).expect(&text);
    let working_p_position = message_text.find(working_p).expect(&text);
    assert!(idle_a_position < working_a_position, "{text}");
    assert!(idle_p_position < working_p_position, "{text}");
    for message in &messages {
        if let (Some(a), Some(p)) = (message.find("- a: idle"), message.find("- p: idle")) {
            assert!(a < p, "{message}");
        }
        if let (Some(a), Some(p)) = (message.find("- a: working"), message.find("- p: working")) {
            assert!(a < p, "{message}");
        }
    }
    assert!(!text.contains("third:"), "{text}");
}

#[test]
fn starting_peers_send_no_activity_notice_until_they_work() {
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
            "w",
            "--harness",
            "generic",
            "--deliver",
            "auto",
            "--watch",
            "a",
            "--watch",
            "b",
            "--activity-working-after",
            "15s",
            "--activity-idle-after",
            "1s",
            "--",
            "sh",
            "-c",
            "stty -echo; printf '\\033[?2004h'; cat",
        ],
    );
    assert!(watcher.status.success(), "stderr: {}", stderr(&watcher));

    let script = dir.path().join("claude-fixture.py");
    std::fs::write(
        &script,
        r#"import os, sys, time

delay = int(sys.argv[1])
active = sys.argv[2] == "active"
time.sleep(delay)
cols = os.get_terminal_size(0).columns
sys.stdout.write(
    "\x1b[?2004h\x1b[?25h\x1b[2J\x1b[1;1Hfixture-ready"
    + "\x1b[3;1H" + "─" * cols
    + "\x1b[5;1H" + "─" * cols
    + "\x1b[4;1H❯\u00a0"
)
sys.stdout.flush()
if active:
    for _ in range(18):
        time.sleep(1)
        sys.stdout.write("\x1b7\x1b[1;20Hx\x1b8")
        sys.stdout.flush()
time.sleep(60)
"#,
    )
    .unwrap();

    for (name, delay, mode) in [("a", "10", "quiet"), ("b", "3", "active")] {
        let args = vec![
            "new".to_owned(),
            "--detach".to_owned(),
            "--name".to_owned(),
            name.to_owned(),
            "--harness".to_owned(),
            "claude".to_owned(),
            "--no-channel".to_owned(),
            "--".to_owned(),
            "sh".to_owned(),
            "-c".to_owned(),
            "exec python3 -u \"$1\" \"$2\" \"$3\"".to_owned(),
            "fixture".to_owned(),
            script.to_str().unwrap().to_owned(),
            delay.to_owned(),
            mode.to_owned(),
        ];
        let peer = run_owned(&home, &args);
        assert!(peer.status.success(), "{name}: stderr: {}", stderr(&peer));
    }

    let deadline = Instant::now() + Duration::from_secs(60);
    let screen = loop {
        let output = run(&home, &["screen", "w"]);
        let text = stdout(&output);
        let notices = text.split("peer activity:").skip(1);
        if notices
            .clone()
            .any(|notice| notice.contains("- b: idle (was working"))
            || Instant::now() >= deadline
        {
            break output;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(screen.status.success(), "stderr: {}", stderr(&screen));
    let text = stdout(&screen);
    assert!(text.contains("- b: idle (was working"), "{text}");
    assert!(!text.contains("- a:"), "{text}");
    assert!(!text.contains("- b: working (was idle"), "{text}");
    let peer_screen = run(&home, &["screen", "a"]);
    assert!(
        peer_screen.status.success(),
        "stderr: {}",
        stderr(&peer_screen)
    );
    assert!(
        stdout(&peer_screen).contains("fixture-ready"),
        "{}",
        stdout(&peer_screen)
    );
}

#[test]
fn held_activity_delivery_keeps_at_most_one_open_message() {
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
            "w",
            "--harness",
            "generic",
            "--deliver",
            "hold",
            "--watch",
            "p",
            "--activity-working-after",
            "1s",
            "--activity-idle-after",
            "1s",
            "--",
            "sh",
            "-c",
            "printf '\\033[?2004h'; sleep 60",
        ],
    );
    assert!(watcher.status.success(), "stderr: {}", stderr(&watcher));
    let peer = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "p",
            "--harness",
            "generic",
            "--",
            "sh",
            "-c",
            "printf '\\033[?2004h'; sleep 15; while :; do printf x; sleep 1; done",
        ],
    );
    assert!(peer.status.success(), "stderr: {}", stderr(&peer));
    let deadline = Instant::now() + Duration::from_secs(30);
    let messages = loop {
        let output = run(&home, &["messages", "--session", "w"]);
        if stdout(&output).contains("activity") || Instant::now() >= deadline {
            break output;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(messages.status.success(), "stderr: {}", stderr(&messages));
    std::thread::sleep(Duration::from_secs(5));
    let messages = run(&home, &["messages", "--session", "w"]);
    let count = stdout(&messages)
        .lines()
        .filter(|line| line.trim_end().ends_with("activity"))
        .count();
    assert_eq!(count, 1, "{}", stdout(&messages));
}

#[test]
fn new_heartbeat_validates_interval_and_requires_watch() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let missing_watch = run(
        &home,
        &[
            "new",
            "--detach",
            "--harness",
            "generic",
            "--heartbeat",
            "30m",
            "--",
            "sleep",
            "60",
        ],
    );
    assert!(!missing_watch.status.success());
    assert!(
        stderr(&missing_watch).contains("heartbeat needs a non-empty watch"),
        "{}",
        stderr(&missing_watch)
    );
    for value in ["0s", "25h"] {
        let invalid = run(
            &home,
            &[
                "new",
                "--detach",
                "--harness",
                "generic",
                "--heartbeat",
                value,
                "--watch",
                "dev",
                "--",
                "sleep",
                "60",
            ],
        );
        assert!(!invalid.status.success());
        assert!(
            stderr(&invalid).contains("invalid heartbeat"),
            "{value}: {}",
            stderr(&invalid)
        );
    }
}

#[test]
fn new_activity_validates_interval_and_requires_watch() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());
    let missing_watch = run(
        &home,
        &[
            "new",
            "--detach",
            "--harness",
            "generic",
            "--activity-working-after",
            "2m",
            "--",
            "sleep",
            "60",
        ],
    );
    assert!(!missing_watch.status.success());
    assert!(
        stderr(&missing_watch).contains("activity needs a non-empty watch"),
        "{}",
        stderr(&missing_watch)
    );
    for value in ["0s", "25h"] {
        let invalid = run(
            &home,
            &[
                "new",
                "--detach",
                "--harness",
                "generic",
                "--activity-idle-after",
                value,
                "--watch",
                "dev",
                "--",
                "sleep",
                "60",
            ],
        );
        assert!(!invalid.status.success());
        assert!(
            stderr(&invalid).contains("invalid activity idle_after"),
            "{value}: {}",
            stderr(&invalid)
        );
    }
}

#[tokio::test]
async fn daemon_new_session_activity_validation_rejects_invalid_requests() {
    use a2amx::harness::Harness;
    use a2amx::wire::{ActivityWatch, Request, Response};

    let (dir, daemon) = common::start_daemon().await;
    let mut client = a2amx::client::Client::connect(dir.path()).await.unwrap();
    let cases = [
        (
            "invalid-none",
            vec!["peer".to_owned()],
            ActivityWatch {
                working_after: None,
                idle_after: None,
            },
            "session invalid-none: activity needs at least one duration",
        ),
        (
            "invalid-range",
            vec!["peer".to_owned()],
            ActivityWatch {
                working_after: Some(86_401),
                idle_after: None,
            },
            "session invalid-range: invalid activity working_after \"86401s\": expected a number followed by s, m or h, from 1s to 24h",
        ),
        (
            "invalid-watch",
            Vec::new(),
            ActivityWatch {
                working_after: Some(1),
                idle_after: None,
            },
            "session invalid-watch: activity needs a non-empty watch",
        ),
    ];
    for (name, watch, activity, expected) in cases {
        let response = client
            .request(Request::NewSession {
                argv: vec!["sleep".into(), "1".into()],
                cols: 40,
                rows: 5,
                cwd: None,
                env: Vec::new(),
                reset: Vec::new(),
                control_from: Vec::new(),
                watch,
                activity: Some(activity),
                heartbeat: None,
                name: Some(name.into()),
                harness: Harness::Generic,
                deliver: None,
                team: None,
                role: None,
                remote: None,
            })
            .await
            .unwrap();
        assert_eq!(
            response,
            Response::Error {
                message: expected.into(),
            }
        );
        let sessions = client.request(Request::List).await.unwrap();
        assert_eq!(
            sessions,
            Response::Sessions {
                sessions: Vec::new()
            }
        );
    }
    daemon.shutdown().await.unwrap();
}

#[test]
fn new_activity_validates_before_connecting_to_daemon() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let output = run(
        &home,
        &[
            "new",
            "--detach",
            "--harness",
            "generic",
            "--activity-working-after",
            "2m",
            "--",
            "sleep",
            "1",
        ],
    );
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("activity needs a non-empty watch"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn not_ready_watcher_does_not_receive_heartbeat() {
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
            "arch",
            "--harness",
            "generic",
            "--watch",
            "dev",
            "--heartbeat",
            "2s",
            "--",
            "sh",
            "-c",
            "sleep 60",
        ],
    );
    assert!(watcher.status.success(), "stderr: {}", stderr(&watcher));
    let peer = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "dev",
            "--harness",
            "generic",
            "--",
            "sh",
            "-c",
            "sleep 60",
        ],
    );
    assert!(peer.status.success(), "stderr: {}", stderr(&peer));
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let messages = run(&home, &["messages", "--session", "arch"]);
        assert!(
            !stdout(&messages).contains("heartbeat"),
            "{}",
            stdout(&messages)
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}
