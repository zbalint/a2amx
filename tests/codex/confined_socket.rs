use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn confined_codex_uses_private_socket_dir_and_delivers() {
    let Some((state_dir, daemon)) = common::start_confined_daemon().await else {
        assert!(
            std::env::var_os("A2AMX_REQUIRE_BWRAP").is_none(),
            "bubblewrap is required for this test"
        );
        return;
    };
    let mut admin = Client::connect(state_dir.path()).await.unwrap();
    let (_, sender_token, sender_addr) = common::new_agent(
        &mut admin,
        state_dir.path(),
        Some("sender"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let work = tempfile::tempdir().unwrap();
    let cwd = work.path().join("workspace");
    std::fs::create_dir_all(&cwd).unwrap();
    let program = cwd.join("codex");
    std::fs::write(&program, FAKE_CODEX).unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    let request = Request::Confine {
        confine: ConfineRequest::default(),
        request: Box::new(Request::NewSession {
            argv: vec![program.to_string_lossy().into_owned()],
            cols: 40,
            rows: 10,
            cwd: Some(cwd.to_string_lossy().into_owned()),
            env: vec![
                ("ARGS_DIR".into(), cwd.to_string_lossy().into_owned()),
                ("PATH".into(), std::env::var("PATH").unwrap_or_default()),
                ("FAKE_CONFINED".into(), "1".into()),
                ("A2AMX_NO_AUTHORIZE_PEERS".into(), "1".into()),
            ],
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            activity: None,
            heartbeat: None,
            name: Some("agent-impl".into()),
            harness: Harness::Codex,
            deliver: None,
            team: None,
            role: None,
            remote: None,
        }),
    };
    let session_dir = state_dir.path().join("run/codex/s2");
    let socket_dir = session_dir.join("sock");
    std::fs::create_dir_all(&socket_dir).unwrap();
    symlink("/tmp", session_dir.join("stale")).unwrap();
    symlink("/tmp", socket_dir.join("stale")).unwrap();
    let created = tokio::spawn(async move { admin.request(request).await.unwrap() });
    let app_link = socket_dir.join("app.sock");
    common::eventually(|| async {
        std::fs::symlink_metadata(&app_link)
            .ok()
            .filter(|metadata| metadata.file_type().is_symlink())
            .map(|_| ())
    })
    .await;
    assert!(std::fs::symlink_metadata(session_dir.join("stale")).is_err());
    assert!(std::fs::symlink_metadata(socket_dir.join("stale")).is_err());
    for path in [&session_dir, &socket_dir] {
        assert_eq!(
            std::fs::symlink_metadata(path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    let fake = FakeCodex::bind(&socket_dir.join("fake"));
    fake.set_threads(&[("thread-1", idle())]);
    fake.auto_enter(true);
    let response = created.await.unwrap();
    assert!(matches!(response, Response::Created { .. }), "{response:?}");
    let server_args = common::eventually(|| async {
        complete(&cwd.join("server.args"))
            .map(|text| text.lines().map(str::to_owned).collect::<Vec<_>>())
    })
    .await;
    let instructions: Vec<_> = server_args
        .iter()
        .filter(|argument| argument.starts_with("developer_instructions="))
        .collect();
    assert_eq!(instructions.len(), 1);
    let notice: String = serde_json::from_str(
        instructions[0]
            .strip_prefix("developer_instructions=")
            .expect("developer instructions prefix"),
    )
    .expect("JSON notice");
    assert!(notice.starts_with("This session is confined.\n"));
    assert!(!notice.contains(a2amx::harness::PEER_AUTHORIZATION_PROMPT));
    let expected_notice = a2amx::confine::notice(
        &a2amx::confine::plan(
            &ConfineRequest::default(),
            Harness::Codex,
            &cwd,
            &Host {
                home: PathBuf::from(std::env::var_os("HOME").expect("HOME")),
                layout: a2amx::layout::Layout::new(state_dir.path().to_path_buf()),
                runtime_dir: PathBuf::from(
                    std::env::var_os("XDG_RUNTIME_DIR").expect("XDG_RUNTIME_DIR"),
                ),
            },
        )
        .expect("expected Codex plan"),
    );
    assert_eq!(notice, expected_notice);
    assert_eq!(
        common::eventually(|| async { complete(&cwd.join("server.cwd")) })
            .await
            .trim(),
        cwd.to_string_lossy()
    );
    assert_eq!(
        common::eventually(|| async { complete(&cwd.join("tui.cwd")) })
            .await
            .trim(),
        cwd.to_string_lossy()
    );
    let guest_socket = format!(
        "unix:///tmp/codex-daemon-{}/app.sock",
        rustix::process::geteuid().as_raw()
    );
    assert_eq!(
        common::eventually(|| async { complete(&cwd.join("tui.args")) }).await,
        format!("--remote\n{guest_socket}\n")
    );
    let host_pidns = std::fs::read_link("/proc/self/ns/pid")
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert_ne!(
        common::eventually(|| async { complete(&cwd.join("server.pidns")) })
            .await
            .trim(),
        host_pidns
    );
    assert_ne!(
        common::eventually(|| async { complete(&cwd.join("tui.pidns")) })
            .await
            .trim(),
        host_pidns
    );
    let mut sender = Client::connect_addr(sender_addr, &sender_token)
        .await
        .unwrap();
    let Response::Accepted { id, .. } = sender
        .request(Request::SendMessage {
            to: "agent-impl@host-a".into(),
            subject: "Parser issue".into(),
            message: BODY.into(),
        })
        .await
        .unwrap()
    else {
        panic!("message was not accepted");
    };
    let submitted = common::eventually(|| async {
        let response = Client::connect(state_dir.path())
            .await
            .unwrap()
            .request(Request::MessageStatus { id: id.clone() })
            .await
            .unwrap();
        match response {
            Response::Status { message } if message.state == "submitted" => Some(message),
            _ => None,
        }
    })
    .await;
    assert_eq!(submitted.state, "submitted");
    common::eventually(|| async {
        fake.turn_starts()
            .first()
            .filter(|turn| turn.text.contains(BODY))
            .map(|_| ())
    })
    .await;
    let server_ns = common::eventually(|| async { complete(&cwd.join("server.pidns")) }).await;
    let tui_ns = common::eventually(|| async { complete(&cwd.join("tui.pidns")) }).await;
    let mut admin = Client::connect(state_dir.path()).await.unwrap();
    assert!(matches!(
        admin
            .request(Request::Kill {
                session: "s2".into(),
                now: true,
            })
            .await
            .unwrap(),
        Response::Ok
    ));
    common::eventually(|| async {
        let response = Client::connect(state_dir.path())
            .await
            .unwrap()
            .request(Request::List)
            .await
            .unwrap();
        match response {
            Response::Sessions { sessions }
                if !sessions
                    .iter()
                    .any(|session| session.harness == Harness::Codex) =>
            {
                Some(())
            }
            _ => None,
        }
    })
    .await;
    common::eventually(|| async {
        let alive = std::fs::read_dir("/proc")
            .ok()
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| name.bytes().all(|byte| byte.is_ascii_digit()))
            .any(|pid| {
                std::fs::read_link(format!("/proc/{pid}/ns/pid"))
                    .ok()
                    .is_some_and(|namespace| {
                        namespace.to_string_lossy().trim() == server_ns.trim()
                            || namespace.to_string_lossy().trim() == tui_ns.trim()
                    })
            });
        (!alive).then_some(())
    })
    .await;
    drop(fake);
    drop(daemon);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn confined_codex_plan_failure_does_not_start_app_server() {
    let Some((state_dir, daemon)) = common::start_confined_daemon().await else {
        assert!(
            std::env::var_os("A2AMX_REQUIRE_BWRAP").is_none(),
            "bubblewrap is required for this test"
        );
        return;
    };
    let mut admin = Client::connect(state_dir.path()).await.unwrap();
    let work = tempfile::tempdir().unwrap();
    let marker = work.path().join("started");
    let fake = work.path().join("codex");
    std::fs::write(&fake, "#!/bin/sh\nprintf started > \"$MARKER\"\nexit 0\n").unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let fake_path = fake.to_string_lossy().into_owned();
    let marker_path = marker.to_string_lossy().into_owned();
    let response = admin
        .request(Request::Confine {
            confine: ConfineRequest {
                write: vec!["relative".into()],
                ..ConfineRequest::default()
            },
            request: Box::new(Request::NewSession {
                argv: vec![fake_path],
                cols: 40,
                rows: 10,
                cwd: Some(work.path().to_string_lossy().into_owned()),
                env: vec![("MARKER".into(), marker_path)],
                reset: vec![],
                control_from: vec![],
                watch: vec![],
                activity: None,
                heartbeat: None,
                name: Some("plan-refused".into()),
                harness: Harness::Codex,
                deliver: Some(Deliver::Hold),
                team: None,
                role: None,
                remote: None,
            }),
        })
        .await;
    assert!(response.is_ok(), "daemon request failed: {response:?}");
    let response = response.unwrap();
    let Response::Error { message } = response else {
        panic!("expected confinement plan refusal: {response:?}");
    };
    assert!(message.contains("relative"), "{message}");
    assert!(
        !marker.exists(),
        "Codex app-server launch marker was created"
    );
    assert!(
        !state_dir.path().join("codex").exists(),
        "Codex app-server state was created despite plan refusal"
    );
    drop(daemon);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn confined_codex_rejects_hostile_socket_links() {
    let Some((state_dir, daemon)) = common::start_confined_daemon().await else {
        assert!(
            std::env::var_os("A2AMX_REQUIRE_BWRAP").is_none(),
            "bubblewrap is required for this test"
        );
        return;
    };
    let mut admin = Client::connect(state_dir.path()).await.unwrap();
    let _ = common::new_agent(
        &mut admin,
        state_dir.path(),
        Some("sender"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let work = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let outside_socket = outside.path().join("outside.sock");
    let listener = std::os::unix::net::UnixListener::bind(&outside_socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let guest_prefix = format!("/tmp/codex-daemon-{}", rustix::process::geteuid().as_raw());
    for (index, (key, value)) in [
        ("FAKE_LINK", format!("{guest_prefix}/../../outside")),
        (
            "FAKE_HASH_LINK",
            outside_socket.to_string_lossy().into_owned(),
        ),
        ("FAKE_LINK", format!("{guest_prefix}/{}", "x".repeat(81))),
        ("FAKE_LINK", "/tmp/elsewhere/fake".into()),
    ]
    .into_iter()
    .enumerate()
    {
        let cwd = work.path().join(format!("case-{index}"));
        std::fs::create_dir_all(&cwd).unwrap();
        let program = cwd.join("codex");
        std::fs::write(&program, FAKE_CODEX).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let request = Request::Confine {
            confine: ConfineRequest::default(),
            request: Box::new(Request::NewSession {
                argv: vec![program.to_string_lossy().into_owned()],
                cols: 40,
                rows: 10,
                cwd: Some(cwd.to_string_lossy().into_owned()),
                env: vec![
                    ("ARGS_DIR".into(), cwd.to_string_lossy().into_owned()),
                    ("PATH".into(), std::env::var("PATH").unwrap_or_default()),
                    (key.into(), value),
                    (
                        "FAKE_EXIT".into(),
                        if index == 2 { "5".into() } else { "1".into() },
                    ),
                ],
                reset: vec![],
                control_from: vec![],
                watch: vec![],
                activity: None,
                heartbeat: None,
                name: Some(format!("hostile-{index}")),
                harness: Harness::Codex,
                deliver: None,
                team: None,
                role: None,
                remote: None,
            }),
        };
        let mut request_client = Client::connect(state_dir.path()).await.unwrap();
        let mut fake_server: Option<FakeCodex> = None;
        let response = if index == 2 {
            let existing_dirs = std::fs::read_dir(state_dir.path().join("run/codex"))
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .collect::<Vec<_>>();
            let pending = tokio::spawn(async move { request_client.request(request).await });
            let socket_dir = common::eventually(|| async {
                std::fs::read_dir(state_dir.path().join("run/codex"))
                    .ok()?
                    .filter_map(Result::ok)
                    .filter_map(|entry| {
                        let session_dir = entry.path();
                        if existing_dirs.contains(&session_dir) {
                            return None;
                        }
                        let socket_dir = session_dir.join("sock");
                        let app_link = socket_dir.join("app.sock");
                        std::fs::symlink_metadata(&app_link)
                            .ok()
                            .filter(|metadata| metadata.file_type().is_symlink())
                            .map(|_| socket_dir)
                    })
                    .max()
            })
            .await;
            fake_server = Some(FakeCodex::bind(&socket_dir.join("short")));
            tokio::time::timeout(Duration::from_secs(8), pending)
                .await
                .unwrap()
                .unwrap()
                .unwrap()
        } else {
            tokio::time::timeout(Duration::from_secs(8), request_client.request(request))
                .await
                .unwrap()
                .unwrap()
        };
        let Response::Error { message } = response else {
            panic!("hostile socket link was accepted: {response:?}");
        };
        assert!(message.contains("unexpected socket link"), "{message}");
        if index == 2 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert_eq!(
                fake_server
                    .as_ref()
                    .expect("short fake listener")
                    .connections(),
                0
            );
        }
    }
    assert!(listener.accept().is_err());
    drop(daemon);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn confined_codex_rejects_host_substituted_socket_dir() {
    let Some((state_dir, daemon)) = common::start_confined_daemon().await else {
        assert!(
            std::env::var_os("A2AMX_REQUIRE_BWRAP").is_none(),
            "bubblewrap is required for this test"
        );
        return;
    };
    let mut admin = Client::connect(state_dir.path()).await.unwrap();
    let _ = common::new_agent(
        &mut admin,
        state_dir.path(),
        Some("sender"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let work = tempfile::tempdir().unwrap();
    let cwd = work.path().join("workspace");
    std::fs::create_dir_all(&cwd).unwrap();
    let program = cwd.join("codex");
    std::fs::write(&program, FAKE_CODEX).unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    let request = Request::Confine {
        confine: ConfineRequest::default(),
        request: Box::new(Request::NewSession {
            argv: vec![program.to_string_lossy().into_owned()],
            cols: 40,
            rows: 10,
            cwd: Some(cwd.to_string_lossy().into_owned()),
            env: vec![
                ("ARGS_DIR".into(), cwd.to_string_lossy().into_owned()),
                ("PATH".into(), std::env::var("PATH").unwrap_or_default()),
                ("A2AMX_NO_AUTHORIZE_PEERS".into(), "1".into()),
                ("FAKE_CONFINED".into(), "1".into()),
            ],
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            activity: None,
            heartbeat: None,
            name: Some("substitute".into()),
            harness: Harness::Codex,
            deliver: None,
            team: None,
            role: None,
            remote: None,
        }),
    };
    std::fs::create_dir_all(state_dir.path().join("run/codex")).unwrap();
    let session_dir = state_dir.path().join("run/codex/s2");
    let socket_dir = session_dir.join("sock");
    let pending = tokio::spawn(async move { admin.request(request).await.unwrap() });
    common::eventually(|| async {
        std::fs::symlink_metadata(socket_dir.join("app.sock"))
            .ok()
            .filter(|metadata| metadata.file_type().is_symlink())
            .map(|_| ())
    })
    .await;
    let original = session_dir.join("sock-original");
    std::fs::rename(&socket_dir, &original).unwrap();
    let substitute = work.path().join("substitute");
    std::fs::create_dir(&substitute).unwrap();
    let fake = FakeCodex::bind(&substitute.join("fake"));
    let guest_fake = format!(
        "/tmp/codex-daemon-{}/fake",
        rustix::process::geteuid().as_raw()
    );
    symlink(guest_fake, substitute.join("app.sock")).unwrap();
    symlink(&substitute, &socket_dir).unwrap();
    let response = tokio::time::timeout(Duration::from_secs(35), pending)
        .await
        .unwrap()
        .unwrap();
    let Response::Error { message } = response else {
        panic!("substituted socket directory was accepted: {response:?}");
    };
    assert!(message.contains("unexpected socket link"), "{message}");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(fake.connections(), 0);
    drop(daemon);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn confined_codex_guest_cannot_replace_socket_directory() {
    let Some((state_dir, daemon)) = common::start_confined_daemon().await else {
        assert!(
            std::env::var_os("A2AMX_REQUIRE_BWRAP").is_none(),
            "bubblewrap is required for this test"
        );
        return;
    };
    let mut admin = Client::connect(state_dir.path()).await.unwrap();
    let _ = common::new_agent(
        &mut admin,
        state_dir.path(),
        Some("sender"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let work = tempfile::tempdir().unwrap();
    let cwd = work.path().join("workspace");
    std::fs::create_dir_all(&cwd).unwrap();
    let program = cwd.join("codex");
    std::fs::write(&program, FAKE_CODEX).unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    let session_dir = state_dir.path().join("run/codex/s2");
    let socket_dir = session_dir.join("sock");
    let request = Request::Confine {
        confine: ConfineRequest::default(),
        request: Box::new(Request::NewSession {
            argv: vec![program.to_string_lossy().into_owned()],
            cols: 40,
            rows: 10,
            cwd: Some(cwd.to_string_lossy().into_owned()),
            env: vec![
                ("ARGS_DIR".into(), cwd.to_string_lossy().into_owned()),
                ("PATH".into(), std::env::var("PATH").unwrap_or_default()),
                ("FAKE_CONFINED".into(), "1".into()),
                ("FAKE_GUEST_MUTATION".into(), "1".into()),
                (
                    "FAKE_SESSION_DIR".into(),
                    session_dir.to_string_lossy().into_owned(),
                ),
            ],
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            activity: None,
            heartbeat: None,
            name: Some("guest-mutation".into()),
            harness: Harness::Codex,
            deliver: None,
            team: None,
            role: None,
            remote: None,
        }),
    };
    let created = tokio::spawn(async move { admin.request(request).await.unwrap() });
    common::eventually(|| async {
        std::fs::symlink_metadata(socket_dir.join("app.sock"))
            .ok()
            .filter(|metadata| metadata.file_type().is_symlink())
            .map(|_| ())
    })
    .await;
    let fake = FakeCodex::bind(&socket_dir.join("fake"));
    let response = created.await.unwrap();
    assert!(matches!(response, Response::Created { .. }), "{response:?}");
    let mutation = common::eventually(|| async { complete(&cwd.join("guest-mutation")) }).await;
    assert!(mutation.contains("parent:\ns2\n"));
    assert!(mutation.contains("codex:\ns2\n"));
    assert!(mutation.contains("mv=fail"));
    assert!(mutation.contains("ln=fail"));
    assert!(mutation.contains("touch=fail"));
    assert!(
        std::fs::symlink_metadata(socket_dir.join("app.sock"))
            .unwrap()
            .is_symlink()
    );
    assert!(fake.connections() >= 1);
    drop(daemon);
}
