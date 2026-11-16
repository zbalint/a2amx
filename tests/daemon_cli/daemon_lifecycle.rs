use super::*;

fn wait_exit(child: &mut Child, limit: Duration) -> Option<std::process::ExitStatus> {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().expect("poll child") {
            return Some(status);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    None
}

#[test]
fn background_start_detaches_and_logs() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);

    let started = Instant::now();
    let output = run(&home, &["daemon", "start"]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(stdout(&output).starts_with("listening on 127.0.0.1:"));
    assert!(run(&home, &["list"]).status.success());
    let mode = std::fs::metadata(home.join("log/daemon.log"))
        .expect("daemon.log exists")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[test]
fn background_refuses_when_already_running() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());

    let second = run(&home, &["daemon", "start"]);

    assert!(!second.status.success());
    assert!(stderr(&second).contains("already running"));
}

#[test]
fn stop_ends_the_daemon() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());

    let stop = run(&home, &["daemon", "stop", "--yes"]);

    assert!(stop.status.success(), "stderr: {}", stderr(&stop));
    assert_eq!(stdout(&stop), "stopped\n");
    assert!(!run(&home, &["list"]).status.success());
}

#[test]
fn stop_without_a_daemon_is_a_no_op() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");

    let stop = run(&home, &["daemon", "stop"]);

    assert!(stop.status.success(), "stderr: {}", stderr(&stop));
    assert_eq!(stdout(&stop), "no daemon running\n");
}

#[test]
fn stop_refuses_to_end_sessions_without_a_terminal() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());
    let created = run(&home, &["new", "--detach", "--", "sh"]);
    assert!(created.status.success(), "stderr: {}", stderr(&created));

    let refused = run(&home, &["daemon", "stop"]);

    assert!(!refused.status.success());
    assert!(stderr(&refused).contains("1 running session"));
    assert!(run(&home, &["list"]).status.success());
    let forced = run(&home, &["daemon", "stop", "--yes"]);
    assert!(forced.status.success(), "stderr: {}", stderr(&forced));
}

#[test]
fn foreground_daemon_honors_stop() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    let mut daemon = a2amx(
        &home,
        &["daemon", "start", "--foreground", "--listen", "127.0.0.1:0"],
    )
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .spawn()
    .expect("spawn foreground daemon");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !run(&home, &["list"]).status.success() {
        assert!(Instant::now() < deadline, "daemon did not come up");
        std::thread::sleep(Duration::from_millis(100));
    }

    let stop = run(&home, &["daemon", "stop", "--yes"]);

    assert!(stop.status.success(), "stderr: {}", stderr(&stop));
    let status = wait_exit(&mut daemon, Duration::from_secs(15));
    if status.is_none() {
        let _ = daemon.kill();
    }
    assert!(status.expect("daemon exited").success());
}

#[test]
fn background_reports_a_startup_failure() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("not-a-directory");
    std::fs::write(&home, b"").unwrap();

    let output = run(&home, &["daemon", "start"]);

    assert!(!output.status.success());
    let message = stderr(&output);
    assert!(
        message.contains("a2amx path is not a directory")
            || message.contains("daemon exited during startup")
            || message.contains("Not a directory"),
        "stderr: {message}"
    );
}

#[test]
fn daemon_status_reports_address_and_session_counts() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());
    let address = std::fs::read_to_string(home.join("run/addr"))
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_owned();
    let cli_version = stdout(&run(&home, &["--version"]));
    let version = cli_version
        .trim()
        .strip_prefix("a2amx ")
        .unwrap()
        .to_owned();
    assert!(!version.is_empty());
    let running = run(&home, &["new", "--detach", "--", "sh", "-c", "sleep 30"]);
    assert!(running.status.success(), "stderr: {}", stderr(&running));
    let exited = run(&home, &["new", "--detach", "--", "sh", "-c", "exit 7"]);
    assert!(exited.status.success(), "stderr: {}", stderr(&exited));
    let listed = screen_contains_until(&home, &["list"], "exited");
    assert!(listed.status.success(), "stderr: {}", stderr(&listed));

    let status = run(&home, &["daemon", "status"]);

    assert!(status.status.success(), "stderr: {}", stderr(&status));
    assert_eq!(
        stdout(&status),
        format!(
            "running\nversion: {version}\nlistening on {address}\nsessions: 1 running, 1 exited\n"
        )
    );
    assert_eq!(stderr(&status), "");
}

#[test]
fn removed_daemon_spellings_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let background = ["--", "background"].concat();
    let top_level_stop = ["st", "op"].concat();
    for args in [
        vec!["daemon"],
        vec!["daemon", background.as_str()],
        vec![top_level_stop.as_str()],
    ] {
        let output = run(&home, &args);
        assert!(!output.status.success(), "accepted {args:?}");
    }
}

#[test]
fn daemon_status_without_a_daemon_is_quiet_failure() {
    let dir = tempfile::tempdir().unwrap();
    let output = run(dir.path(), &["daemon", "status"]);

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stdout(&output), "not running\n");
    assert_eq!(stderr(&output), "");
}

#[test]
fn new_rejects_the_daemon_sender_name() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());
    let output = run(
        &home,
        &["new", "--detach", "--name", "a2amx-daemon", "--", "true"],
    );
    assert!(!output.status.success(), "{}", stdout(&output));
    assert!(
        stderr(&output).contains("reserved for the daemon"),
        "{}",
        stderr(&output)
    );
    let self_watch = run(
        &home,
        &[
            "new", "--detach", "--name", "self", "--watch", "self", "--", "true",
        ],
    );
    assert!(!self_watch.status.success(), "{}", stdout(&self_watch));
    assert!(
        stderr(&self_watch).contains("watches itself"),
        "{}",
        stderr(&self_watch)
    );
}

#[test]
fn foreground_rejects_invalid_daemon_config_without_starting() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    std::fs::create_dir_all(home.join("config")).unwrap();
    std::fs::write(home.join("config/daemon.toml"), "limits.rate = 0\n").unwrap();
    let mut child = a2amx(&home, &["daemon", "start", "--foreground"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let status = wait_exit(&mut child, Duration::from_secs(10));
    if status.is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    assert!(
        status.is_some(),
        "foreground daemon did not reject invalid config"
    );
    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert!(stderr(&output).contains("limits.rate"));
    assert!(!home.join("run/addr").exists());
}

#[test]
fn background_rejects_invalid_daemon_config_before_child_or_log() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    std::fs::create_dir_all(home.join("config")).unwrap();
    std::fs::write(home.join("config/daemon.toml"), "limits.rate = 0\n").unwrap();
    let output = run(&home, &["daemon", "start"]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("limits.rate"));
    assert!(!home.join("run/addr").exists());
    assert!(!home.join("log/daemon.log").exists());
}

#[test]
fn foreground_start_creates_daemon_template() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    let mut command = a2amx(&home, &["daemon", "start", "--foreground"]);
    command
        .env_remove("A2AMX_PREFIX")
        .env_remove("A2AMX_VIEWER_AFTER");
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !home.join("run/addr").exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        std::fs::read_to_string(home.join("config/daemon.toml"))
            .unwrap()
            .contains("# owner_away")
    );
    assert!(
        std::fs::read_to_string(home.join("config/client.toml"))
            .unwrap()
            .contains("# prefix")
    );
    let _ = run(&home, &["daemon", "stop", "--yes"]);
    let _ = child.wait();
}

#[test]
fn invalid_client_config_stops_regular_commands_but_not_confine_list() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    std::fs::create_dir_all(home.join("config")).unwrap();
    std::fs::write(home.join("config/client.toml"), "prefix = \"x\"\n").unwrap();

    let mut list = a2amx(&home, &["list"]);
    list.env_remove("A2AMX_PREFIX");
    list.env_remove("A2AMX_VIEWER_AFTER");
    let output = list.output().unwrap();
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("client.toml: key prefix:"),
        "{}",
        stderr(&output)
    );
    assert!(!home.join("run/addr").exists());

    let mut confine = a2amx(&home, &["confine", "list"]);
    confine.env_remove("A2AMX_PREFIX");
    confine.env_remove("A2AMX_VIEWER_AFTER");
    let output = confine.output().unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(!home.join("run/addr").exists());
}

#[test]
fn configured_daemon_starts_and_stops() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    std::fs::create_dir_all(home.join("config")).unwrap();
    std::fs::write(
        home.join("config/daemon.toml"),
        "[limits]\nretention = \"3d\"\n",
    )
    .unwrap();
    let _cleanup = Cleanup(&home);
    let started = run(&home, &["daemon", "start"]);
    assert!(started.status.success(), "{}", stderr(&started));
    assert!(run(&home, &["daemon", "status"]).status.success());
    assert!(run(&home, &["daemon", "stop", "--yes"]).status.success());
}

#[test]
fn config_show_is_read_only_on_fresh_root() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let mut command = a2amx(&home, &["config", "show"]);
    command.env_remove("A2AMX_OWNER_AWAY_SECS");
    command.env_remove("A2AMX_PREFIX");
    command.env_remove("A2AMX_VIEWER_AFTER");
    let output = command.output().unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    let expected = format!(
        "daemon.toml: {} (absent)\n\
listen = [\"127.0.0.1:0\"]  (default)\n\
host_name = (unset)  (default)\n\
owner_away = \"1m\"  (default)\n\
limits.max_pending_per_recipient = 50  (default)\n\
limits.rate_per_minute = 20  (default)\n\
limits.human_inbound_per_minute = 20  (default)\n\
limits.human_outbound_per_minute = 30  (default)\n\
limits.max_stored_messages = 10000  (default)\n\
limits.retention = \"7d\"  (default)\n\
limits.lease = \"1d\"  (default)\n\
limits.code_ttl = \"10m\"  (default)\n\
\n\
gateway.toml: {} (absent)\n\
guild_id = (unset)  (default)\n\
control_category = \"a2amx\"  (default)\n\
control_channel = \"control\"  (default)\n\
max_members = 2  (default)\n\
sync = \"15s\"  (default)\n\
idle = \"1s\"  (default)\n\
member_check = \"1m\"  (default)\n\
reconnect = \"5s\"  (default)\n\
\n\
client.toml: {} (absent)\n\
prefix = \"C-b\"  (default)\n\
viewer_after = \"5m\"  (default)\n",
        home.join("config/daemon.toml").display(),
        home.join("config/gateway.toml").display(),
        home.join("config/client.toml").display(),
    );
    let text = stdout(&output);
    assert_eq!(text, expected);
    assert_eq!(text.lines().count(), 26);
    assert!(!home.join("config/daemon.toml").exists());
    assert!(!home.join("config/gateway.toml").exists());
    assert!(!home.join("config/client.toml").exists());
    assert!(!home.join("config").exists());
    assert!(!home.exists());
}

#[test]
fn config_show_reads_owner_away_environment() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let output = a2amx(&home, &["config", "show"])
        .env("A2AMX_OWNER_AWAY_SECS", "5")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stdout(&output)
            .lines()
            .any(|line| line == "owner_away = \"5s\"  (env)")
    );
    assert!(!home.exists());
}
