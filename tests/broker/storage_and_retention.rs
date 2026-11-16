use super::*;

#[tokio::test]
async fn retention_zero_and_database_permissions() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon_in(
        dir.path(),
        Some("host-a"),
        Limits {
            retention_secs: 0,
            ..Limits::default()
        },
    )
    .await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_sender_id, _sender_token, mut sender) = session(
        &mut admin,
        dir.path(),
        Some("sender"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let (_recipient_id, _recipient_token, _recipient) = session(
        &mut admin,
        dir.path(),
        Some("recipient"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    accepted(
        send(&mut sender, "recipient@host-a", "one", "body").await,
        "m_1",
        Some("deliver_hold"),
    );
    assert_secure_database(dir.path());
    daemon.shutdown().await.unwrap();

    let daemon = common::start_daemon_in(
        dir.path(),
        Some("host-a"),
        Limits {
            retention_secs: 0,
            ..Limits::default()
        },
    )
    .await;
    let mut new_admin = Client::connect(dir.path()).await.unwrap();
    match new_admin
        .request(Request::ListMessages {
            session: None,
            state: None,
        })
        .await
        .unwrap()
    {
        Response::Messages { messages } => assert!(messages.is_empty()),
        response => panic!("unexpected response: {response:?}"),
    }
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn abrupt_binary_restart_recovers_pending_as_daemon_restarted() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = DaemonChild::start(dir.path(), None);
    let address = common::eventually(|| async {
        std::fs::read_to_string(dir.path().join("run/addr"))
            .ok()
            .and_then(|text| text.lines().next()?.parse::<SocketAddr>().ok())
    })
    .await;
    let admin_token = std::fs::read_to_string(dir.path().join("run/admin.token"))
        .unwrap()
        .trim()
        .to_owned();
    let mut admin = Client::connect_addr(address, &admin_token).await.unwrap();
    let (_sender_id, _sender_token, mut sender) = session(
        &mut admin,
        dir.path(),
        Some("sender"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let (_recipient_id, _recipient_token, _recipient) = session(
        &mut admin,
        dir.path(),
        Some("recipient"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    accepted(
        send(&mut sender, "recipient@host-a", "one", "body").await,
        "m_1",
        Some("deliver_hold"),
    );
    child.kill();

    let daemon = common::start_daemon_in(dir.path(), Some("host-a"), Limits::default()).await;
    let mut new_admin = Client::connect(dir.path()).await.unwrap();
    let message = match status(&mut new_admin, "m_1").await {
        Response::Status { message } => message,
        response => panic!("unexpected response: {response:?}"),
    };
    assert_eq!(message.state, "undeliverable");
    assert_eq!(message.detail.as_deref(), Some("daemon_restarted"));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn storage_failure_returns_internal_without_acceptance_or_content_leak() {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_, _, mut sender) = session(
        &mut admin,
        dir.path(),
        Some("sender"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let _ = session(
        &mut admin,
        dir.path(),
        Some("recipient"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let wal = dir.path().join("state/messages.db-wal");
    let original = dir.path().join("state/messages.db-wal.original");
    std::fs::rename(&wal, &original).unwrap();
    std::os::unix::fs::symlink("messages.db-wal", &wal).unwrap();
    let response = send(
        &mut sender,
        "recipient@host-a",
        "private subject",
        "private body",
    )
    .await;
    std::fs::remove_file(&wal).unwrap();
    std::fs::rename(&original, &wal).unwrap();
    let Response::Failed { code, message } = response else {
        panic!("storage failure was accepted")
    };
    assert_eq!(code, "internal");
    assert!(!message.contains("private subject"));
    assert!(!message.contains("private body"));
    let Response::Messages { messages } = admin
        .request(Request::ListMessages {
            session: None,
            state: None,
        })
        .await
        .unwrap()
    else {
        panic!("message list")
    };
    assert!(
        messages.is_empty(),
        "a failed acceptance must store nothing"
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn migrates_v1_database_and_preserves_submitted_evidence() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("state")).unwrap();
    let database = rusqlite::Connection::open(dir.path().join("state/messages.db")).unwrap();
    database
        .execute_batch(
            "CREATE TABLE messages (
               seq INTEGER PRIMARY KEY AUTOINCREMENT,
               boot TEXT NOT NULL,
               sender_session TEXT NOT NULL,
               sender_address TEXT NOT NULL,
               recipient_session TEXT NOT NULL,
               recipient_address TEXT NOT NULL,
               subject TEXT NOT NULL,
               body TEXT NOT NULL,
               state TEXT NOT NULL,
               detail TEXT,
               accepted_at INTEGER NOT NULL,
               updated_at INTEGER NOT NULL
             );
             CREATE INDEX messages_recipient
               ON messages (boot, recipient_session, state, seq);
             CREATE TABLE attempts (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               message_seq INTEGER NOT NULL REFERENCES messages(seq),
               started_at INTEGER NOT NULL,
               outcome TEXT NOT NULL,
               detail TEXT,
               finished_at INTEGER
             );
             PRAGMA user_version = 1;
             INSERT INTO messages
               (seq, boot, sender_session, sender_address, recipient_session,
                recipient_address, subject, body, state, detail, accepted_at, updated_at)
             VALUES
               (1, 'boot-v1', 's1', 'sender@host-a', 's2', 'recipient@host-a',
                'subject', 'body', 'submitted', NULL, 2000000000, 2000000000);
             INSERT INTO attempts
               (message_seq, started_at, outcome, detail, finished_at)
             VALUES (1, 2000000000, 'submitted', NULL, 2000000000);",
        )
        .unwrap();
    drop(database);

    let daemon = common::start_daemon_in(dir.path(), Some("host-a"), Limits::default()).await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let messages = match admin
        .request(Request::ListMessages {
            session: None,
            state: None,
        })
        .await
        .unwrap()
    {
        Response::Messages { messages } => messages,
        response => panic!("unexpected response: {response:?}"),
    };
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].id, "m_1");
    assert_eq!(messages[0].state, "submitted");
    assert_eq!(messages[0].evidence.as_deref(), Some("write_complete"));

    daemon.shutdown().await.unwrap();
    let database = rusqlite::Connection::open(dir.path().join("state/messages.db")).unwrap();
    let version: i64 = database
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 2);
}

#[tokio::test]
async fn daemon_refuses_a_future_database_schema() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("state")).unwrap();
    let database = rusqlite::Connection::open(dir.path().join("state/messages.db")).unwrap();
    database.pragma_update(None, "user_version", 3).unwrap();
    drop(database);
    let bwrap_dir = dir.path().join("bwrap-path");
    std::fs::create_dir_all(&bwrap_dir).unwrap();
    let error = a2amx::daemon::Daemon::start(a2amx::daemon::DaemonConfig {
        root: dir.path().to_owned(),
        listen: vec!["127.0.0.1:0".parse().unwrap()],
        host_name: Some("host-a".into()),
        limits: Limits::default(),
        bwrap_path: Some(bwrap_dir.into_os_string()),
        env_path: None,
    })
    .await
    .err()
    .expect("future schema must fail startup");
    assert_eq!(
        error.to_string(),
        "unsupported messages.db schema version 3"
    );
    assert!(
        !dir.path().join("run/admin.token").exists(),
        "a failed start must not leave a token behind"
    );
}

#[tokio::test]
async fn retention_includes_the_exact_nonzero_expiry_boundary() {
    let (dir, bootstrap) = common::start_daemon().await;
    bootstrap.shutdown().await.unwrap();
    let path = dir.path().to_owned();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/frozen_realtime.c");
    let mut process = tokio::task::spawn_blocking(move || {
        let library = path.join("frozen_realtime.so");
        let compiled = Command::new("cc")
            .args(["-shared", "-fPIC", "-Wall", "-Wextra", "-Werror", "-o"])
            .arg(&library).arg(fixture).arg("-ldl").output().unwrap();
        assert!(compiled.status.success(), "clock fixture compiler: {}", String::from_utf8_lossy(&compiled.stderr));
        let database = rusqlite::Connection::open(path.join("state/messages.db")).unwrap();
        // Persisted startup input only. At fixed now=2000000000 and the default
        // 604800-second TTL: m_1 is exactly expired, m_2 has one second left,
        // m_4 is strictly expired, and recovery gives m_3 a fresh updated_at.
        database.execute_batch(
            "INSERT INTO messages
             (seq,boot,sender_session,sender_address,recipient_session,recipient_address,subject,body,state,accepted_at,updated_at)
             VALUES
             (1,'0000000000000000','s1','agent-plan@host-a','s2','agent-review@host-a','At expiry','Fixture','submitted',1999395200,1999395200),
             (2,'0000000000000000','s1','agent-plan@host-a','s2','agent-review@host-a','Before expiry','Fixture','cancelled',1999395201,1999395201),
             (3,'0000000000000000','s1','agent-plan@host-a','s2','agent-review@host-a','Interrupted','Fixture','delivering',1999395199,1999395199),
             (4,'0000000000000000','s1','agent-plan@host-a','s2','agent-review@host-a','Expired','Fixture','submitted',1999395199,1999395199);
             INSERT INTO attempts (message_seq,started_at,finished_at,outcome) VALUES
             (1,1999395200,1999395200,'submitted'),
             (3,1999395199,NULL,'started'),
             (4,1999395199,1999395199,'submitted');"
        ).unwrap();
        drop(database);
        DaemonChild::start(&path, Some(&library))
    }).await.unwrap();
    let mut admin = common::eventually(|| async { Client::connect(dir.path()).await.ok() }).await;
    let Response::Messages { messages } = admin
        .request(Request::ListMessages {
            session: None,
            state: None,
        })
        .await
        .unwrap()
    else {
        panic!("message list")
    };
    // This witness catches a missing preload rather than misclassifying fixture
    // setup failure as a retention-boundary reproduction.
    assert!(
        messages.iter().all(|message| message.id != "m_4"),
        "fixed clock must expire a strictly old row"
    );
    assert_eq!(
        messages
            .iter()
            .map(|message| message.id.as_str())
            .collect::<Vec<_>>(),
        ["m_2", "m_3"]
    );
    assert_eq!(messages[0].state, "cancelled");
    assert_eq!(messages[1].state, "undeliverable");
    assert_eq!(messages[1].detail.as_deref(), Some("daemon_restarted"));
    let mut child = process.0.take().unwrap();
    let status = tokio::task::spawn_blocking(move || {
        let pid = rustix::process::Pid::from_raw(child.id() as i32).unwrap();
        rustix::process::kill_process(pid, rustix::process::Signal::TERM).unwrap();
        child.wait().unwrap()
    })
    .await
    .unwrap();
    assert!(status.success());
}
