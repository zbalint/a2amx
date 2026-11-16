use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn new_list_attach_detach_and_literal_prefix_use_real_terminal() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;

    let (created, code) = run_cli(
        dir.path(),
        &["new", "--detach", "--", "sh", "-c", "printf ready; cat"],
    )?;
    assert_eq!(code, 0);
    assert!(created.contains("s1"), "new output: {created:?}");

    let (listed, code) = run_cli(dir.path(), &["list"])?;
    assert_eq!(code, 0);
    assert!(listed.contains("ID  NAME"), "list output: {listed:?}");
    assert!(
        listed.contains("s1") && listed.contains("running"),
        "list output: {listed:?}"
    );

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("ready", WAIT)?;
    attached.send(b"typed\n")?;
    attached.wait_for_text("typed", WAIT)?;
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    assert!(
        attached
            .raw_output()
            .windows(b"\x1b[?1049l".len())
            .any(|window| window == b"\x1b[?1049l")
    );

    let (after_detach, code) = run_cli(dir.path(), &["list"])?;
    assert_eq!(code, 0);
    assert!(after_detach.contains("s1") && after_detach.contains("running"));

    // Ctrl-B twice is a literal 0x02, not a prefix command. The child
    // prints the received byte in hex and stays alive so the screen is observable.
    let (created, code) = run_cli(
        dir.path(),
        &[
            "new",
            "--detach",
            "--",
            "sh",
            "-c",
            "stty raw -echo; dd if=/dev/tty bs=1 count=1 2>/dev/null | od -An -t x1; sleep 30",
        ],
    )?;
    assert_eq!(code, 0);
    assert!(created.contains("s2"), "new output: {created:?}");
    let mut literal = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s2"]), dir.path())?;
    literal.send(&[2, 2])?;
    literal.wait_for_text("02", WAIT)?;
    literal.send(&[2, b'd'])?;
    literal.wait_for_text("[detached from s2]", WAIT)?;
    assert_eq!(literal.wait_exit(WAIT)?, 0);
    Ok(())
}

#[test]
fn attach_without_terminal_is_rejected_with_status_one() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let output = run_binary(dir.path(), &["attach", "s1"], &[])?;
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stderr)?,
        "a2amx: attach needs a terminal on stdin\n"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn home_precedence_and_nonterminal_new_use_default_size() -> anyhow::Result<()> {
    let (first_dir, _first_daemon) = common::start_daemon().await;
    let second_dir = tempfile::tempdir()?;
    let bwrap_dir = second_dir.path().join("bwrap-path");
    std::fs::create_dir_all(&bwrap_dir)?;
    let _second_daemon = a2amx::daemon::Daemon::start(a2amx::daemon::DaemonConfig {
        root: second_dir.path().to_path_buf(),
        listen: vec!["127.0.0.1:0".parse()?],
        host_name: None,
        limits: a2amx::messaging::Limits::default(),
        bwrap_path: Some(bwrap_dir.into_os_string()),
        env_path: None,
    })
    .await?;
    let first_home = first_dir.path().to_string_lossy().into_owned();
    let second_home = second_dir.path().to_string_lossy().into_owned();
    let environment = [
        ("A2AMX_HOME", first_home.as_str()),
        ("XDG_STATE_HOME", second_home.as_str()),
    ];

    let created = run_binary(
        first_dir.path(),
        &["new", "--detach", "--", "sh", "-c", "sleep 30"],
        &environment,
    )?;
    assert_eq!(created.status.code(), Some(0));
    assert_eq!(String::from_utf8(created.stdout)?, "s1\n");
    let generated_name = String::from_utf8(created.stderr)?;
    let generated_name = generated_name
        .strip_prefix("name: ")
        .expect("generated name")
        .trim();
    let name_width = generated_name.len().max("NAME".len());
    let expected_header = format!(
        "ID  {:<name_width$}  HARNESS  CONFINE  STATE    UPTIME  ACTIVITY  IN-STATE  ATTACHED  PENDING  HELD  QUOTA  SIZE",
        "NAME",
        name_width = name_width,
    );
    let listed = run_binary(first_dir.path(), &["list"], &environment)?;
    assert_eq!(listed.status.code(), Some(0));
    let listing = String::from_utf8(listed.stdout)?;
    let header = listing.lines().next().unwrap_or_default();
    assert_eq!(header, expected_header);
    let cells = common::list_cells(&listing, "s1");
    assert_eq!(cells.len(), 13);
    assert_eq!(cells[0], "s1");
    assert_ne!(cells[1], "-");
    a2amx::messaging::validate_name(cells[1])?;
    assert_eq!(&cells[2..5], ["generic", "off", "running"]);
    assert_eq!(cells[6], "working");
    for index in [5, 7] {
        let bytes = cells[index].as_bytes();
        assert!(bytes.len() >= 2);
        assert!(
            bytes[..bytes.len() - 1]
                .iter()
                .all(|byte| byte.is_ascii_digit())
        );
        assert!(matches!(bytes.last(), Some(b's' | b'm' | b'h')));
    }

    let explicit = run_binary(second_dir.path(), &["list"], &environment)?;
    assert_eq!(explicit.status.code(), Some(0));
    let explicit_listing = String::from_utf8(explicit.stdout)?;
    assert!(explicit_listing.starts_with(
        "ID  NAME  HARNESS  CONFINE  STATE  UPTIME  ACTIVITY  IN-STATE  ATTACHED  PENDING  HELD  QUOTA  SIZE\n",
    ));
    assert!(!explicit_listing.contains("s1"));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn custom_prefix_and_sigwinch_resize_reach_attached_session() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let script = "stty size; trap 'stty size' WINCH; while :; do read line; done";
    let (_, code) = run_cli(dir.path(), &["new", "--detach", "--", "sh", "-c", script])?;
    assert_eq!(code, 0);

    let mut attached = PtyHarness::spawn_with_env(
        &cli_args(dir.path(), &["attach", "s1"]),
        dir.path(),
        &[("A2AMX_PREFIX", "C-a")],
    )?;
    attached.wait_for_text("22 80", WAIT)?;
    attached.resize(40, 10)?;
    attached.wait_for_text("8 40", WAIT)?;
    attached.send(&[1, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    assert!(
        attached
            .raw_output()
            .windows(b"\x1b[?1049l".len())
            .any(|window| window == b"\x1b[?1049l")
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn client_file_prefix_reaches_attached_session() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    std::fs::write(dir.path().join("config/client.toml"), "prefix = \"C-x\"\n")?;
    let script = "stty size; while :; do read line; done";
    let (_, code) = run_cli(dir.path(), &["new", "--detach", "--", "sh", "-c", script])?;
    assert_eq!(code, 0);

    let mut args = vec![
        "-u".to_owned(),
        "A2AMX_PREFIX".to_owned(),
        "-u".to_owned(),
        "A2AMX_VIEWER_AFTER".to_owned(),
        env!("CARGO_BIN_EXE_a2amx").to_owned(),
    ];
    args.extend(cli_args(dir.path(), &["attach", "s1"]));
    let mut attached = PtyHarness::spawn_program("env", &args, dir.path(), &[])?;
    attached.wait_for_text("24 80", WAIT)?;
    attached.send(&[24, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn output_failure_restores_the_outer_terminal_before_returning_an_error() -> anyhow::Result<()>
{
    let (dir, _daemon) = common::start_daemon().await;
    let (_, code) = run_cli(dir.path(), &["new", "--detach", "--", "cat"])?;
    assert_eq!(code, 0);
    let before = dir.path().join("before");
    let after = dir.path().join("after");
    let script = "stty -g > \"$BEFORE\"; \"$A2AMX_BIN\" --home \"$A2AMX_HOME\" attach s1 > /dev/full; status=$?; stty -g > \"$AFTER\"; printf 'status=%s\\n' \"$status\"";
    let mut terminal = PtyHarness::spawn_program(
        "sh",
        &["-c".into(), script.into()],
        dir.path(),
        &[
            ("A2AMX_BIN", env!("CARGO_BIN_EXE_a2amx")),
            ("BEFORE", before.to_str().unwrap()),
            ("AFTER", after.to_str().unwrap()),
        ],
    )?;
    terminal.wait_for_text("status=1", WAIT)?;
    assert_eq!(terminal.wait_exit(WAIT)?, 0);
    assert_eq!(std::fs::read(before)?, std::fs::read(after)?);
    assert!(terminal.screen_text().contains("a2amx:"));
    Ok(())
}

#[test]
fn invalid_prefix_reports_the_named_reason_as_an_a2amx_error() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let output = run_binary(dir.path(), &["--prefix", "C-m", "list"], &[])?;
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.starts_with("a2amx:"));
    assert!(stderr.contains("CR"));
    Ok(())
}
async fn wait_for_file(path: &Path, lines: usize) -> String {
    let path = path.to_owned();
    eventually(move || {
        let path = path.clone();
        async move {
            let content = std::fs::read_to_string(path).ok()?;
            (content.lines().count() == lines).then_some(content)
        }
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn claude_new_wires_mcp_and_optional_peer_authorization() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let first = tempfile::NamedTempFile::new_in(dir.path())?;
    let first_path = first.path().to_string_lossy().into_owned();
    let first_output = run_binary(
        dir.path(),
        &[
            "new",
            "--detach",
            "--harness",
            "claude",
            "--no-channel",
            "--",
            "sh",
            "-c",
            "printf '%s\\n' \"$@\" >\"$OUT\"; sleep 30",
            "sh",
        ],
        &[("OUT", &first_path)],
    )?;
    assert_eq!(first_output.status.code(), Some(0));
    let first_args = wait_for_file(first.path(), 8).await;
    let first_args: Vec<&str> = first_args.lines().collect();
    assert_eq!(first_args.len(), 8);
    assert_eq!(first_args[0], "--mcp-config");
    let config: serde_json::Value = serde_json::from_str(first_args[1])?;
    let expected_exe = std::fs::canonicalize(env!("CARGO_BIN_EXE_a2amx"))?
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        config["mcpServers"]["a2amx"]["command"],
        serde_json::Value::String(expected_exe.clone())
    );
    assert_eq!(
        config["mcpServers"]["a2amx"]["args"],
        serde_json::json!(["mcp"])
    );
    assert_eq!(first_args[2], "--allowedTools");
    assert_eq!(
        first_args[3],
        "mcp__a2amx__list_agents,mcp__a2amx__send_message,mcp__a2amx__message_status,mcp__a2amx__reset_session"
    );
    assert_eq!(first_args[4], "--append-system-prompt");
    assert_eq!(first_args[5], a2amx::harness::PEER_AUTHORIZATION_PROMPT);
    assert_eq!(first_args[6], "--settings");
    let settings: serde_json::Value = serde_json::from_str(first_args[7])?;
    assert_eq!(
        settings["hooks"]["UserPromptSubmit"][0]["hooks"][0]["command"],
        format!("'{expected_exe}' hook")
    );

    let second = tempfile::NamedTempFile::new_in(dir.path())?;
    let second_path = second.path().to_string_lossy().into_owned();
    let second_output = run_binary(
        dir.path(),
        &[
            "new",
            "--detach",
            "--harness",
            "claude",
            "--no-channel",
            "--no-authorize-peers",
            "--",
            "sh",
            "-c",
            "printf '%s\\n' \"$@\" >\"$OUT\"; sleep 30",
            "sh",
        ],
        &[("OUT", &second_path)],
    )?;
    assert_eq!(second_output.status.code(), Some(0));
    let second_args = wait_for_file(second.path(), 6).await;
    let second_args: Vec<&str> = second_args.lines().collect();
    assert_eq!(second_args.len(), 6);
    assert_eq!(
        second_args,
        vec![
            "--mcp-config",
            first_args[1],
            "--allowedTools",
            "mcp__a2amx__list_agents,mcp__a2amx__send_message,mcp__a2amx__message_status,mcp__a2amx__reset_session",
            "--settings",
            first_args[7],
        ]
    );
    Ok(())
}

#[tokio::test]
async fn output_during_backpressured_snapshot_reaches_live_attachment_without_redraw()
-> anyhow::Result<()> {
    use a2amx::wire::{ClientFrame, ServerFrame, encode_frame};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    tokio::time::timeout(WAIT, async {
        let (dir, daemon) = common::start_daemon().await;
        let gate_path = dir.path().join("output-gate");
        let listener = tokio::net::UnixListener::bind(&gate_path)?;
        let script = r#"
import os, socket, sys, tty
tty.setraw(0)
gate = socket.socket(socket.AF_UNIX)
gate.connect(sys.argv[1])
def synced_write(data):
    sys.stdout.buffer.write(data + b'\x1b[6n')
    sys.stdout.buffer.flush()
    while os.read(0, 1) != b'R':
        pass
pair = b'\x1b[38;2;1;2;3mX\x1b[38;2;4;5;6mX'
synced_write(pair * 131072)
gate.sendall(b'ready')
gate.recv(1)
synced_write(b'\x1b[1;1H\x1b[0mOUTPUT-AFTER-SNAPSHOT')
gate.sendall(b'printed')
gate.recv(1)
"#;
        let mut control = Client::connect(dir.path()).await?;
        let Response::Created { session } = control
            .request(Request::NewSession {
                argv: vec![
                    "python3".into(),
                    "-c".into(),
                    script.into(),
                    gate_path.to_string_lossy().into_owned(),
                ],
                cols: 512,
                rows: 512,
                cwd: None,
                env: vec![],
                reset: vec![],
                control_from: vec![],
                watch: vec![],
                name: Some("render-regression".into()),
                harness: Harness::Generic,
                deliver: Some(Deliver::Hold),
                heartbeat: None,
                activity: None,
                team: None,
                role: None,
                remote: None,
            })
            .await?
        else {
            panic!("session creation");
        };
        let (mut gate, _) = listener.accept().await?;
        let mut ready = [0; 5];
        gate.read_exact(&mut ready).await?;
        assert_eq!(&ready, b"ready");

        // Alternating RGB cells make the snapshot exceed Linux's TCP send buffer.
        // A small receive window leaves the daemon sending it while the child writes.
        let socket = tokio::net::TcpSocket::new_v4()?;
        socket.set_recv_buffer_size(16384)?;
        let mut attachment = socket.connect(daemon.addrs()[0]).await?;
        let token = std::fs::read_to_string(dir.path().join("run/admin.token"))?;
        for (request, expected) in [
            (Request::Hello { token }, Response::Ok),
            (
                Request::Attach {
                    session,
                    force: false,
                    cols: 512,
                    rows: 512,
                    status: false,
                    top_rows: 0,
                    viewer: false,
                },
                Response::Attached,
            ),
        ] {
            attachment
                .write_all(&encode_frame(&serde_json::to_vec(&request)?)?)
                .await?;
            let length = attachment.read_u32().await?;
            let mut payload = vec![0; length as usize];
            attachment.read_exact(&mut payload).await?;
            assert_eq!(serde_json::from_slice::<Response>(&payload)?, expected);
        }
        gate.write_all(b"p").await?;
        let mut printed = [0; 7];
        gate.read_exact(&mut printed).await?;
        assert_eq!(&printed, b"printed");
        // Let the already-woken delivery observer consume the shared notification.
        assert!(matches!(
            control.request(Request::List).await?,
            Response::Sessions { .. }
        ));
        tokio::task::yield_now().await;
        tokio::time::timeout(WAIT, async {
            loop {
                let length = attachment.read_u32().await?;
                let mut payload = vec![0; length as usize];
                attachment.read_exact(&mut payload).await?;
                let ServerFrame::Data(data) = ServerFrame::decode(&payload)? else {
                    panic!("child remains live");
                };
                if data
                    .windows(b"OUTPUT-AFTER-SNAPSHOT".len())
                    .any(|window| window == b"OUTPUT-AFTER-SNAPSHOT")
                {
                    break;
                }
            }
            Ok::<_, anyhow::Error>(())
        })
        .await??;
        attachment
            .write_all(&encode_frame(&ClientFrame::Detach.encode())?)
            .await?;
        gate.write_all(b"x").await?;
        daemon.shutdown().await?;
        Ok::<_, anyhow::Error>(())
    })
    .await?
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn switch_waits_for_old_attachment_eof_with_delayed_fake_daemon() -> anyhow::Result<()> {
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
    attached.send(b"\x1b[<0;30;1M\x1b[<0;30;1m")?;
    attached.wait_for_text("BETA", WAIT)?;
    attached.send(&[2, b'd'])?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    assert!(
        !daemon.early_switch.load(Ordering::SeqCst),
        "target attach arrived before old stream EOF"
    );
    server.abort();
    let _ = server.await;
    Ok(())
}
