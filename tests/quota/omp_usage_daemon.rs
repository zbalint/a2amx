use super::*;

const OMP_USAGE_FIXTURE: &str = r#"{"reports":[{"provider":"openai-codex","limits":[
    {"window":{"id":"5h"},"amount":{"remainingFraction":0.97}},
    {"window":{"id":"7d"},"amount":{"remainingFraction":0.05}}
]}]}"#;

const OMP_USAGE_MULTI_FIXTURE: &str = r#"{"reports":[
    {"provider":"openai-codex","limits":[
        {"window":{"id":"5h"},"amount":{"remainingFraction":0.97}},
        {"window":{"id":"7d"},"amount":{"remainingFraction":0.05}}
    ]},
    {"provider":"opencode-go","limits":[
        {"window":{"id":"5h"},"amount":{"remainingFraction":0.0}},
        {"window":{"id":"7d"},"amount":{"remainingFraction":0.0}}
    ]}
]}"#;

fn fake_executable(dir: &Path, name: &str, usage_body: &str) -> PathBuf {
    let path = dir.join(name);
    let script =
        format!("#!/bin/sh\nif [ \"$1\" = usage ]; then\n{usage_body}\nelse\nsleep 60\nfi\n");
    fs::write(&path, script).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    path
}

fn bridged_fake_executable(dir: &Path, usage: &str) -> PathBuf {
    let path = dir.join("omp");
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = usage ]; then\nprintf '%s\\n' '{usage}'; exit 0\nelse\nprintf '%s\\n%s\\n' \"$A2AMX_TOKEN\" \"$A2AMX_ADDR\" > \"$OUT\"\nprintf '\\033[?2004h'\ncat\nfi\n"
    );
    fs::write(&path, script).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    path
}

async fn bridged_omp_session(
    admin: &mut Client,
    daemon_address: std::net::SocketAddr,
    name: &str,
    executable: &Path,
    cwd: &Path,
    output: &Path,
    provider: &str,
) -> (String, BridgeLink) {
    let response = admin
        .request(Request::NewSession {
            argv: vec![executable.to_string_lossy().into_owned()],
            cols: 169,
            rows: 51,
            cwd: Some(cwd.to_string_lossy().into_owned()),
            env: vec![("OUT".into(), output.to_string_lossy().into_owned())],
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            heartbeat: None,
            activity: None,
            team: None,
            role: None,
            remote: None,
            name: Some(name.into()),
            harness: Harness::Omp,
            deliver: Some(Deliver::Hold),
        })
        .await
        .unwrap();
    let Response::Created { session } = response else {
        panic!("session creation failed: {response:?}");
    };
    let token = common::eventually(|| async {
        let text = fs::read_to_string(output).ok()?;
        let mut lines = text.lines();
        let token = lines.next()?.to_owned();
        let _address = lines.next()?;
        (token.len() == 64).then_some(token)
    })
    .await;
    let mut bridge = Client::connect_addr(daemon_address, &token)
        .await
        .unwrap()
        .bridge()
        .await
        .unwrap();
    bridge
        .send(BridgeUp::Hello {
            protocol: 1,
            omp_version: "18.4.5".into(),
            missing: vec![],
        })
        .await
        .unwrap();
    assert!(matches!(
        bridge.recv().await.unwrap(),
        Some(BridgeDown::Ready)
    ));
    bridge
        .send(BridgeUp::State {
            idle: true,
            pending: false,
            draft: false,
            queue: Some(0),
            host_pending: Some(false),
            provider: Some(provider.to_owned()),
        })
        .await
        .unwrap();
    (session, bridge)
}

async fn direct_omp_session(
    admin: &mut Client,
    name: &str,
    executable: &Path,
    cwd: &Path,
) -> String {
    let response = admin
        .request(Request::NewSession {
            argv: vec![executable.to_string_lossy().into_owned()],
            cols: 169,
            rows: 51,
            cwd: Some(cwd.to_string_lossy().into_owned()),
            env: vec![],
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            heartbeat: None,
            activity: None,
            team: None,
            role: None,
            remote: None,
            name: Some(name.into()),
            harness: Harness::Omp,
            deliver: Some(Deliver::Hold),
        })
        .await
        .unwrap();
    let Response::Created { session } = response else {
        panic!("session creation failed: {response:?}");
    };
    session
}

#[tokio::test]
async fn list_and_list_agents_report_omp_usage_percentages() {
    let fake_dir = tempdir().unwrap();
    let output = fake_dir.path().join("bridge-state");
    let executable = bridged_fake_executable(fake_dir.path(), OMP_USAGE_FIXTURE);
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (id, mut bridge) = bridged_omp_session(
        &mut admin,
        daemon.addrs()[0],
        "agent-omp-usage",
        &executable,
        fake_dir.path(),
        &output,
        "openai-codex",
    )
    .await;

    let quota = quota_of(&mut admin, &id).await;
    assert_eq!(
        quota,
        QuotaInfo {
            five_hour_left: Some(97),
            weekly_left: Some(5),
            ..Default::default()
        }
    );
    let Response::Agents { agents, .. } = admin.request(Request::ListAgents).await.unwrap() else {
        panic!("list_agents failed");
    };
    let agent = agents
        .iter()
        .find(|agent| agent.address.starts_with("agent-omp-usage@"))
        .unwrap();
    assert_eq!(agent.quota, Some(quota));

    bridge
        .send(BridgeUp::State {
            idle: true,
            pending: false,
            draft: false,
            queue: Some(0),
            host_pending: Some(false),
            provider: None,
        })
        .await
        .unwrap();
    let mut cleared = false;
    for _ in 0..100 {
        if summary(&mut admin, &id).await.quota.is_none() {
            cleared = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(cleared, "provider omission did not clear List quota");
    let Response::Agents { agents, .. } = admin.request(Request::ListAgents).await.unwrap() else {
        panic!("list_agents failed");
    };
    let agent = agents
        .iter()
        .find(|agent| agent.address.starts_with("agent-omp-usage@"))
        .unwrap();
    assert_eq!(agent.quota, None);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn send_message_uses_recipient_omp_provider_quota() {
    let fake_dir = tempdir().unwrap();
    let executable = bridged_fake_executable(fake_dir.path(), OMP_USAGE_MULTI_FIXTURE);
    let open_output = fake_dir.path().join("open-bridge-state");
    let glm_output = fake_dir.path().join("glm-bridge-state");
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (open_id, _open_bridge) = bridged_omp_session(
        &mut admin,
        daemon.addrs()[0],
        "agent-omp-open",
        &executable,
        fake_dir.path(),
        &open_output,
        "openai-codex",
    )
    .await;
    let (glm_id, _glm_bridge) = bridged_omp_session(
        &mut admin,
        daemon.addrs()[0],
        "agent-omp-glm",
        &executable,
        fake_dir.path(),
        &glm_output,
        "opencode-go",
    )
    .await;
    assert_eq!(
        quota_of(&mut admin, &open_id).await,
        QuotaInfo {
            five_hour_left: Some(97),
            weekly_left: Some(5),
            ..Default::default()
        }
    );
    assert_eq!(
        quota_of(&mut admin, &glm_id).await,
        QuotaInfo {
            five_hour_left: Some(0),
            weekly_left: Some(0),
            ..Default::default()
        }
    );

    let (_, sender_token, sender_address) = common::new_agent(
        &mut admin,
        dir.path(),
        Some("agent-omp-sender"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let mut sender = Client::connect_addr(sender_address, &sender_token)
        .await
        .unwrap();

    let Response::Accepted {
        recipient_quota: open_quota,
        ..
    } = sender
        .request(Request::SendMessage {
            to: "agent-omp-open@host-a".into(),
            subject: "s".into(),
            message: "m".into(),
        })
        .await
        .unwrap()
    else {
        panic!("expected healthy recipient acceptance");
    };
    let Response::Accepted {
        recipient_quota: glm_quota,
        ..
    } = sender
        .request(Request::SendMessage {
            to: "agent-omp-glm@host-a".into(),
            subject: "s".into(),
            message: "m".into(),
        })
        .await
        .unwrap()
    else {
        panic!("expected exhausted recipient acceptance");
    };
    assert_eq!(open_quota, None);
    assert_eq!(glm_quota.as_deref(), Some("5h and weekly quota exhausted"));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn failed_omp_usage_poll_is_not_retried_every_check() {
    let fake_dir = tempdir().unwrap();
    let executable = fake_executable(
        fake_dir.path(),
        "omp",
        "printf '%s\\n' attempt >> counter\nexit 7",
    );
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let id = direct_omp_session(
        &mut admin,
        "agent-omp-failure",
        &executable,
        fake_dir.path(),
    )
    .await;

    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(summary(&mut admin, &id).await.quota, None);
    assert_eq!(
        fs::read_to_string(fake_dir.path().join("counter"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    assert!(matches!(
        admin.request(Request::List).await.unwrap(),
        Response::Sessions { .. }
    ));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn omp_usage_poll_keeps_absolute_session_executable() {
    let fake_dir = tempdir().unwrap();
    let marker = fake_dir.path().join("usage-ran");
    let usage = format!("printf ran > usage-ran\nprintf '%s\\n' '{OMP_USAGE_FIXTURE}'");
    let executable = fake_executable(fake_dir.path(), "omp", &usage);
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let id = direct_omp_session(
        &mut admin,
        "agent-omp-absolute",
        &executable,
        fake_dir.path(),
    )
    .await;

    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if marker.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("absolute executable was polled");
    assert_eq!(summary(&mut admin, &id).await.quota, None);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn slow_omp_usage_poll_does_not_block_list() {
    let fake_dir = tempdir().unwrap();
    let executable = fake_executable(fake_dir.path(), "omp", "printf started > started\nsleep 60");
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let id = direct_omp_session(&mut admin, "agent-omp-slow", &executable, fake_dir.path()).await;

    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if fake_dir.path().join("started").exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("usage poll started");
    let response = tokio::time::timeout(Duration::from_secs(1), admin.request(Request::List))
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(response, Response::Sessions { .. }));
    assert_eq!(summary(&mut admin, &id).await.quota, None);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn omp_usage_timeout_kills_process_group() {
    let fake_dir = tempdir().unwrap();
    let executable = fake_executable(
        fake_dir.path(),
        "omp",
        "(sleep 25; printf survived > grandchild_survived) &\nprintf started > started\nsleep 60",
    );
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let _id = direct_omp_session(
        &mut admin,
        "agent-omp-timeout",
        &executable,
        fake_dir.path(),
    )
    .await;

    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if fake_dir.path().join("started").exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("usage poll started");
    tokio::time::sleep(Duration::from_secs(27)).await;
    assert!(!fake_dir.path().join("grandchild_survived").exists());
    daemon.shutdown().await.unwrap();
}
#[tokio::test]
async fn omp_wrapper_named_bun_is_not_polled() {
    let fake_dir = tempdir().unwrap();
    let executable = fake_executable(
        fake_dir.path(),
        "bun",
        "printf '%s\\n' attempt >> counter\nexit 0",
    );
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let id = direct_omp_session(
        &mut admin,
        "agent-omp-wrapper",
        &executable,
        fake_dir.path(),
    )
    .await;

    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(summary(&mut admin, &id).await.quota, None);
    assert_eq!(
        fs::read_to_string(fake_dir.path().join("counter")).unwrap_or_default(),
        ""
    );
    daemon.shutdown().await.unwrap();
}
