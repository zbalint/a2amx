use super::*;

#[test]
fn shutdown_wire_defaults_graceful_and_omits_false_now() {
    let kill: Request = serde_json::from_str(r#"{"type":"kill","session":"s1"}"#).unwrap();
    assert_eq!(
        kill,
        Request::Kill {
            session: "s1".into(),
            now: false,
        }
    );
    let shutdown = Request::Shutdown { now: false };
    assert_eq!(
        serde_json::to_string(&shutdown).unwrap(),
        r#"{"type":"shutdown"}"#
    );
}
#[tokio::test]
async fn state_is_owner_only_and_exclusively_locked() {
    let (dir, daemon) = common::start_daemon().await;
    assert_eq!(
        std::fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777,
        0o755
    );
    for file in ["run/admin.token", "run/addr"] {
        assert_eq!(
            std::fs::metadata(dir.path().join(file))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    for file in ["run/gateway.token", "run/addr"] {
        let path = dir.path().join(file);
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let second = Daemon::start(DaemonConfig {
        root: dir.path().to_owned(),
        listen: vec!["127.0.0.1:0".parse().unwrap()],
        host_name: None,
        limits: a2amx::messaging::Limits::default(),
        bwrap_path: Some(dir.path().join("bwrap-path").into_os_string()),
        env_path: None,
    })
    .await;
    assert_eq!(
        second.err().unwrap().to_string(),
        "another a2amx daemon is running for this a2amx directory"
    );
    daemon.shutdown().await.unwrap();
    assert!(!dir.path().join("run/addr").exists());
    assert!(!dir.path().join("run/link/addr").exists());
}

#[tokio::test]
async fn fresh_daemon_tree_uses_layout_paths() {
    let (dir, daemon) = common::start_daemon().await;
    let root_entries = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<std::collections::BTreeSet<_>>();
    let expected = ["config", "log", "run", "state"]
        .into_iter()
        .map(std::ffi::OsString::from)
        .collect::<std::collections::BTreeSet<_>>();
    assert!(expected.is_subset(&root_entries));
    for file in [
        "run/addr",
        "run/admin.token",
        "run/daemon.lock",
        "run/link/addr",
        "run/link/gateway.token",
        "state/messages.db",
    ] {
        assert!(dir.path().join(file).is_file(), "missing {file}");
    }
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn kill_removes_running_session_and_shutdown_disconnects_attachments() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    let id = create(&mut client, &["sh", "-c", "sleep 30"], vec![]).await;
    assert_eq!(
        client
            .request(Request::Kill {
                session: id,
                now: true,
            })
            .await
            .unwrap(),
        Response::Ok
    );
    assert_eq!(list(&mut client).await, vec![]);
    let id = create(&mut client, &["cat"], vec![]).await;
    let mut attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, false, 40, 5)
        .await
        .unwrap();
    next(&mut attachment).await;
    daemon.shutdown().await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(10), attachment.recv())
            .await
            .unwrap()
            .unwrap()
            .is_none()
    );
    assert!(!dir.path().join("addr").exists());
}

#[tokio::test]
async fn missing_or_refused_daemon_has_the_exact_start_hint() {
    let temp = tempfile::tempdir().unwrap();
    let expected = "cannot reach the a2amx daemon: start it with `a2amx daemon start`";
    assert_eq!(
        Client::connect(temp.path())
            .await
            .err()
            .unwrap()
            .to_string(),
        expected
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    assert_eq!(
        Client::connect_addr(address, "unused")
            .await
            .err()
            .unwrap()
            .to_string(),
        expected
    );
}

#[tokio::test]
async fn partial_bind_failure_releases_addresses_and_lock_and_parents_are_private() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("parent/state");
    let occupied = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = occupied.local_addr().unwrap();
    let bwrap_dir = home.join("bwrap-path");
    assert!(
        Daemon::start(DaemonConfig {
            root: home.clone(),
            listen: vec!["127.0.0.1:0".parse().unwrap(), address],
            host_name: None,
            limits: a2amx::messaging::Limits::default(),
            bwrap_path: Some(bwrap_dir.clone().into_os_string()),
            env_path: None,
        })
        .await
        .is_err()
    );
    assert!(!home.join("addr").exists());
    assert!(!home.join("run/admin.token").exists());
    assert!(!home.join("run/gateway.token").exists());
    assert!(!home.join("run/link/gateway.token").exists());
    for path in [home.clone(), temp.path().join("parent")] {
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    std::fs::create_dir_all(&bwrap_dir).unwrap();
    let daemon = Daemon::start(DaemonConfig {
        root: home,
        listen: vec!["127.0.0.1:0".parse().unwrap()],
        host_name: None,
        limits: a2amx::messaging::Limits::default(),
        bwrap_path: Some(bwrap_dir.into_os_string()),
        env_path: None,
    })
    .await
    .unwrap();
    assert_ne!(daemon.addrs()[0].port(), 0);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn kill_escalates_ignored_hangup_and_reports_the_signal_exit() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    let id = create(
        &mut client,
        &["sh", "-c", "trap '' HUP; printf ready; exec sleep 30"],
        vec![],
    )
    .await;
    let mut attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, false, 40, 5)
        .await
        .unwrap();
    let mut emu = Emulator::new(Size { cols: 40, rows: 5 });
    wait_text(&mut attachment, &mut emu, "ready").await;
    assert_eq!(
        client
            .request(Request::Kill {
                session: id,
                now: true
            })
            .await
            .unwrap(),
        Response::Ok
    );
    loop {
        match next(&mut attachment).await {
            ServerFrame::Exit(code) => {
                assert_eq!(code, 137);
                break;
            }
            ServerFrame::Data(_) => {}
            frame => panic!("{frame:?}"),
        }
    }
    assert_eq!(list(&mut client).await, vec![]);
    daemon.shutdown().await.unwrap();
}

#[test]
fn newly_created_parents_are_private_even_with_a_restrictive_umask() {
    use std::os::unix::process::CommandExt;

    if let Some(path) = std::env::var_os("A2AMX_TEST_PRIVATE_STATE") {
        let path = std::path::PathBuf::from(path);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let bwrap_dir = path.join("bwrap-path");
        let daemon = runtime
            .block_on(Daemon::start(DaemonConfig {
                root: path.clone(),
                listen: vec!["127.0.0.1:0".parse().unwrap()],
                host_name: None,
                limits: a2amx::messaging::Limits::default(),
                bwrap_path: Some(bwrap_dir.clone().into_os_string()),
                env_path: None,
            }))
            .unwrap();
        std::fs::create_dir_all(&bwrap_dir).unwrap();
        for directory in [path.clone(), path.parent().unwrap().to_owned()] {
            assert_eq!(
                std::fs::metadata(directory).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        for file in ["run/admin.token", "run/addr"] {
            assert_eq!(
                std::fs::metadata(path.join(file))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        runtime.block_on(daemon.shutdown()).unwrap();
        return;
    }

    let root = tempfile::tempdir().unwrap();
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "daemon_lifecycle::newly_created_parents_are_private_even_with_a_restrictive_umask",
        ])
        .env("A2AMX_TEST_PRIVATE_STATE", root.path().join("parent/state"))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    // SAFETY: after fork the child invokes only the non-allocating umask syscall
    // before exec; the parallel parent tests' process-global mask is never changed.
    unsafe {
        command.pre_exec(|| {
            rustix::process::umask(rustix::fs::Mode::all());
            Ok(())
        });
    }
    let child = command.spawn().unwrap();
    let pid = rustix::process::Pid::from_raw(child.id() as i32).unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(child.wait_with_output());
    });
    let result = receiver.recv_timeout(Duration::from_secs(10));
    if result.is_err() {
        let _ = rustix::process::kill_process(pid, rustix::process::Signal::KILL);
    }
    let output = result
        .expect("isolated daemon check within deadline")
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}
