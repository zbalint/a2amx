use super::*;

#[test]
fn kill_running_session_requires_confirmation_without_a_terminal() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());
    let created = run(&home, &["new", "--detach", "--", "sh", "-c", "sleep 30"]);
    assert!(created.status.success(), "stderr: {}", stderr(&created));
    let id = stdout(&created).trim().to_owned();

    let refused = run(&home, &["kill", &id]);

    assert!(!refused.status.success());
    assert_eq!(
        stderr(&refused),
        format!("a2amx: refusing to kill: session {id} is running; pass --yes to end it\n")
    );
    let listing = stdout(&run(&home, &["list"]));
    assert!(listing.contains(&id));
    assert!(listing.contains("running"));
    let forced = run(&home, &["kill", &id, "--yes"]);
    assert!(forced.status.success(), "stderr: {}", stderr(&forced));
    assert_eq!(stdout(&forced), "");
}

#[test]
fn kill_declined_at_the_prompt_leaves_the_session_running() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());
    let created = run(&home, &["new", "--detach", "--", "sh", "-c", "sleep 30"]);
    assert!(created.status.success(), "stderr: {}", stderr(&created));
    let id = stdout(&created).trim().to_owned();

    let args = vec![
        "--home".to_owned(),
        home.to_string_lossy().into_owned(),
        "kill".to_owned(),
        id.clone(),
    ];
    let mut terminal = PtyHarness::spawn(&args, &home).unwrap();
    terminal
        .wait_for_text(
            &format!("Killing {id} ends a running session. Continue? [y/N]"),
            Duration::from_secs(10),
        )
        .unwrap();
    terminal.send(b"n\n").unwrap();
    terminal
        .wait_for_text("not killed\n", Duration::from_secs(10))
        .unwrap();
    assert_eq!(terminal.wait_exit(Duration::from_secs(10)).unwrap(), 0);

    let listing = run(&home, &["list"]);
    assert!(listing.status.success(), "stderr: {}", stderr(&listing));
    let listing = stdout(&listing);
    assert!(listing.contains(&id));
    assert!(listing.contains("running"));
}

#[test]
fn kill_exited_session_skips_confirmation() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());
    let created = run(&home, &["new", "--detach", "--", "sh", "-c", "exit 7"]);
    assert!(created.status.success(), "stderr: {}", stderr(&created));
    let id = stdout(&created).trim().to_owned();
    let listed = screen_contains_until(&home, &["list"], "exited");
    assert!(listed.status.success(), "stderr: {}", stderr(&listed));

    let killed = run(&home, &["kill", &id]);

    assert!(killed.status.success(), "stderr: {}", stderr(&killed));
}

struct ExitFixture {
    _dir: tempfile::TempDir,
    home: std::path::PathBuf,
    record: std::path::PathBuf,
}

impl ExitFixture {
    fn start(mode: &str, harness: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let fixture = Self {
            home: dir.path().join("state"),
            record: dir.path().join("bytes"),
            _dir: dir,
        };
        let script = fixture._dir.path().join("exit.py");
        std::fs::write(
            &script,
            r#"import os, sys, time, tty
tty.setraw(0)
record = open(sys.argv[1], "wb", buffering=0)
mode = sys.argv[2]
harness = sys.argv[3]
cols = os.get_terminal_size().columns
sys.stdout.write("\x1b[?2004h\x1b[?25h\x1b[2J\x1b[1;1Hfixture-ready")
if mode != "not-ready":
    if harness == "agy":
        sys.stdout.write("\x1b[3;1H" + "─" * cols + "\x1b[5;1H" + "─" * cols + "\x1b[4;1H> \x1b[4;3H")
    else:
        sys.stdout.write("\x1b[3;1H" + "─" * cols + "\x1b[5;1H" + "─" * cols + "\x1b[4;1H❯\u00a0")
sys.stdout.flush()
first = None
while True:
    byte = os.read(0, 1)
    record.write(byte)
    if byte == b"\x04":
        if mode == "first":
            sys.exit(0)
        if mode == "two":
            now = time.monotonic()
            if first is not None and now - first <= 0.6:
                sys.exit(0)
            first = now
"#,
        )
        .unwrap();
        let path = if harness == "agy" {
            let bin = fixture.home.join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            let installed = bin.join("a2amx");
            std::fs::write(&installed, "#!/bin/sh\nexit 0\n").unwrap();
            std::fs::set_permissions(&installed, std::fs::Permissions::from_mode(0o700)).unwrap();
            let agy_dir = fixture._dir.path().join("agy-bin");
            std::fs::create_dir(&agy_dir).unwrap();
            let agy = agy_dir.join("agy");
            let state = fixture._dir.path().join("agy-installed");
            std::fs::write(
                &agy,
                format!(
                    "#!/bin/sh\ncase \"$1 $2\" in\n'plugin validate') exit 0;;\n'plugin list') if test -f '{}'; then printf '[{{\"name\":\"a2amx\",\"version\":\"{}\",\"path\":\"%s\"}}]' \"$(cat '{}')\"; else printf '[]'; fi;;\n'plugin install') printf '%s' \"$3\" > '{}';;\nesac\n",
                    state.display(),
                    env!("CARGO_PKG_VERSION"),
                    state.display(),
                    state.display()
                ),
            )
            .unwrap();
            std::fs::set_permissions(&agy, std::fs::Permissions::from_mode(0o700)).unwrap();
            Some(format!(
                "{}:{}",
                agy_dir.display(),
                std::env::var("PATH").unwrap_or_default()
            ))
        } else {
            None
        };
        let mut started = a2amx(&fixture.home, &["daemon", "start"]);
        started.env("RUST_LOG", "info").env("NO_COLOR", "1");
        if let Some(path) = &path {
            started.env("PATH", path);
        }
        let started = started.output().unwrap();
        assert!(started.status.success(), "{}", stderr(&started));
        let args = [
            "new",
            "--detach",
            "--name",
            "worker",
            "--harness",
            harness,
            "--no-channel",
            "--",
            "sh",
            "-c",
            "exec python3 -u \"$1\" \"$2\" \"$3\" \"$4\"",
            "fixture",
            script.to_str().unwrap(),
            fixture.record.to_str().unwrap(),
            mode,
            harness,
        ];
        let mut create = a2amx(&fixture.home, &args);
        if let Some(path) = &path {
            create.env("PATH", path);
        }
        let created = create.stdin(Stdio::null()).output().unwrap();
        assert!(created.status.success(), "{}", stderr(&created));
        let screen = screen_contains_until(&fixture.home, &["screen", "worker"], "fixture-ready");
        assert!(
            stdout(&screen).contains("fixture-ready"),
            "{}",
            stderr(&screen)
        );
        fixture
    }

    fn bytes(&self) -> Vec<u8> {
        std::fs::read(&self.record).unwrap()
    }

    fn log(&self) -> String {
        std::fs::read_to_string(self.home.join("log/daemon.log")).unwrap()
    }
}

impl Drop for ExitFixture {
    fn drop(&mut self) {
        let _ = run(&self.home, &["daemon", "stop", "--yes"]);
    }
}

#[test]
fn kill_gracefully_exits_on_ctrl_d() {
    let fixture = ExitFixture::start("first", "claude");
    let killed = run(&fixture.home, &["kill", "worker", "--yes"]);
    assert!(killed.status.success(), "{}", stderr(&killed));
    assert_eq!(fixture.bytes(), b"\x04");
    assert!(
        fixture.log().contains("outcome=\"exited\""),
        "{}",
        fixture.log()
    );
    assert!(!stdout(&run(&fixture.home, &["list"])).contains("worker"));
}

#[test]
fn kill_now_skips_ctrl_d() {
    let fixture = ExitFixture::start("first", "claude");
    let killed = run(&fixture.home, &["kill", "worker", "--yes", "--now"]);
    assert!(killed.status.success(), "{}", stderr(&killed));
    assert!(fixture.bytes().is_empty());
    assert!(
        !fixture.log().contains("outcome=\"exited\""),
        "{}",
        fixture.log()
    );
}

#[test]
fn kill_sends_claude_two_key_exit_sequence() {
    let fixture = ExitFixture::start("two", "claude");
    let started = Instant::now();
    let killed = run(&fixture.home, &["kill", "worker", "--yes"]);
    assert!(killed.status.success(), "{}", stderr(&killed));
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "two-key exit took {:?}",
        started.elapsed()
    );
    assert_eq!(fixture.bytes(), b"\x04\x04");
    assert!(
        fixture.log().contains("outcome=\"exited\""),
        "{}",
        fixture.log()
    );
}

#[test]
fn kill_sends_agy_two_key_exit_sequence() {
    let fixture = ExitFixture::start("two", "agy");
    let started = Instant::now();
    let killed = run(&fixture.home, &["kill", "worker", "--yes"]);
    assert!(killed.status.success(), "{}", stderr(&killed));
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "two-key exit took {:?}",
        started.elapsed()
    );
    assert_eq!(fixture.bytes(), b"\x04\x04");
    assert!(
        fixture.log().contains("outcome=\"exited\""),
        "{}",
        fixture.log()
    );
}

#[test]
fn kill_falls_back_after_graceful_timeout() {
    let fixture = ExitFixture::start("ignore", "claude");
    let started = Instant::now();
    let killed = run(&fixture.home, &["kill", "worker", "--yes"]);
    assert!(killed.status.success(), "{}", stderr(&killed));
    assert!(started.elapsed() >= Duration::from_secs(10));
    assert!(fixture.bytes().starts_with(b"\x04\x04"));
    assert!(
        fixture.log().contains("outcome=\"timeout\""),
        "{}",
        fixture.log()
    );
}

#[test]
fn kill_skips_ctrl_d_when_claude_is_not_ready() {
    let fixture = ExitFixture::start("not-ready", "claude");
    let killed = run(&fixture.home, &["kill", "worker", "--yes"]);
    assert!(killed.status.success(), "{}", stderr(&killed));
    assert!(fixture.bytes().is_empty());
    assert!(
        fixture
            .log()
            .contains("outcome=\"skipped\" code=\"not_ready\""),
        "{}",
        fixture.log()
    );
}

#[test]
fn kill_generic_session_uses_hup_without_ctrl_d() {
    let fixture = ExitFixture::start("first", "generic");
    let killed = run(&fixture.home, &["kill", "worker", "--yes"]);
    assert!(killed.status.success(), "{}", stderr(&killed));
    assert!(fixture.bytes().is_empty());
    assert!(
        !fixture.log().contains("graceful exit"),
        "{}",
        fixture.log()
    );
}

#[test]
fn daemon_stop_gracefully_exits_idle_session() {
    let fixture = ExitFixture::start("first", "claude");
    let stopped = run(&fixture.home, &["daemon", "stop", "--yes"]);
    assert!(stopped.status.success(), "{}", stderr(&stopped));
    assert_eq!(fixture.bytes(), b"\x04");
    assert!(
        fixture.log().contains("outcome=\"exited\""),
        "{}",
        fixture.log()
    );
}

#[test]
fn daemon_stop_now_skips_ctrl_d() {
    let fixture = ExitFixture::start("first", "claude");
    let stopped = run(&fixture.home, &["daemon", "stop", "--now", "--yes"]);
    assert!(stopped.status.success(), "{}", stderr(&stopped));
    assert!(fixture.bytes().is_empty());
    assert!(
        !fixture.log().contains("outcome=\"exited\""),
        "{}",
        fixture.log()
    );
}

#[test]
fn team_down_gracefully_exits_idle_session() {
    let fixture = ExitFixture::start("first", "claude");
    let down = run(&fixture.home, &["team", "down", "worker"]);
    assert!(down.status.success(), "{}", stderr(&down));
    assert_eq!(fixture.bytes(), b"\x04");
    assert!(
        fixture.log().contains("outcome=\"exited\""),
        "{}",
        fixture.log()
    );
}

#[test]
fn team_down_now_skips_ctrl_d() {
    let fixture = ExitFixture::start("first", "claude");
    let down = run(&fixture.home, &["team", "down", "--now", "worker"]);
    assert!(down.status.success(), "{}", stderr(&down));
    assert!(fixture.bytes().is_empty());
    assert!(
        !fixture.log().contains("outcome=\"exited\""),
        "{}",
        fixture.log()
    );
}
