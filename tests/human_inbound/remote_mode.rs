use super::*;

pub fn run_binary(
    home: &Path,
    cwd: &Path,
    user_home: &Path,
    runtime: &Path,
    args: &[&str],
) -> Output {
    Command::new(env!("CARGO_BIN_EXE_a2amx"))
        .arg("--home")
        .arg(home)
        .args(args)
        .current_dir(cwd)
        .env("HOME", user_home)
        .env("XDG_RUNTIME_DIR", runtime)
        .output()
        .expect("run a2amx")
}
#[tokio::test]
async fn inbound_requires_explicit_both_mode_and_stores_nothing_when_refused() {
    let Some((dir, daemon)) = start_confined_daemon().await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let _send = create_confined(
        &mut admin,
        fixture.path(),
        "send-target",
        Some(RemoteMode::Send),
    )
    .await;
    let _unset = create_confined(&mut admin, fixture.path(), "unset-target", None).await;
    let refused = new_session_request_with_remote(
        Some("unconfined-remote"),
        &["sleep", "30"],
        Some(RemoteMode::Both),
    );
    assert_eq!(
        error_message(admin.request(refused).await.unwrap()),
        "remote messaging needs a confined session"
    );
    let Response::Sessions { sessions } = admin.request(Request::List).await.unwrap() else {
        panic!("expected session listing");
    };
    assert!(
        sessions
            .iter()
            .all(|session| session.name.as_deref() != Some("unconfined-remote"))
    );
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    for target in ["send-target@host-a", "unset-target@host-a"] {
        let response = gateway
            .request(Request::HumanInbound {
                to: target.into(),
                body: "message".into(),
            })
            .await
            .unwrap();
        assert_eq!(failed_code(response.clone()), code::NOT_PERMITTED);
        assert_eq!(
            failed_message(response),
            "remote messages to this session are not enabled"
        );
    }
    let Response::Messages { messages } = admin
        .request(Request::ListMessages {
            session: None,
            state: None,
        })
        .await
        .unwrap()
    else {
        panic!("expected message listing");
    };
    assert!(messages.iter().all(|message| {
        !["send-target@host-a", "unset-target@host-a"].contains(&message.to.as_str())
    }));
    daemon.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_remote_requires_confinement_and_creates_nothing() {
    let (dir, daemon) = common::start_daemon().await;
    let cwd = tempfile::tempdir().unwrap();
    let user_home = tempfile::tempdir().unwrap();
    let runtime = tempfile::tempdir().unwrap();
    for mode in ["maybe", ""] {
        let invalid = run_binary(
            dir.path(),
            cwd.path(),
            user_home.path(),
            runtime.path(),
            &["new", "--remote", mode, "--", "true"],
        );
        let stderr = String::from_utf8_lossy(&invalid.stderr);
        assert!(!invalid.status.success());
        assert!(stderr.contains("send"), "{stderr}");
        assert!(stderr.contains("both"), "{stderr}");
    }
    let output = run_binary(
        dir.path(),
        cwd.path(),
        user_home.path(),
        runtime.path(),
        &[
            "new",
            "--detach",
            "--name",
            "unconfined-remote",
            "--remote",
            "both",
            "--",
            "sleep",
            "30",
        ],
    );
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("remote messaging needs a confined session")
    );
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let Response::Sessions { sessions } = admin.request(Request::List).await.unwrap() else {
        panic!("expected session listing");
    };
    assert!(
        sessions
            .iter()
            .all(|session| session.name.as_deref() != Some("unconfined-remote"))
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_remote_mode_reaches_new_session_and_controls_inbound_direction() {
    let Some((dir, daemon)) = start_confined_daemon().await else {
        return;
    };
    let cwd = tempfile::tempdir().unwrap();
    let user_home = tempfile::tempdir().unwrap();
    let runtime = tempfile::tempdir().unwrap();
    for mode in ["send", "both"] {
        let name = format!("cli-{mode}");
        let output = run_binary(
            dir.path(),
            cwd.path(),
            user_home.path(),
            runtime.path(),
            &[
                "new",
                "--detach",
                "--name",
                &name,
                "--confine",
                "--remote",
                mode,
                "--",
                "sleep",
                "30",
            ],
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    assert_eq!(
        failed_message(
            gateway
                .request(Request::HumanInbound {
                    to: "cli-send@host-a".into(),
                    body: "message".into(),
                })
                .await
                .unwrap()
        ),
        "remote messages to this session are not enabled"
    );
    assert_eq!(
        failed_message(
            gateway
                .request(Request::HumanInbound {
                    to: "cli-both@host-a".into(),
                    body: "message".into(),
                })
                .await
                .unwrap()
        ),
        "remote messaging is not active for this team; start it at the computer"
    );
    daemon.shutdown().await.unwrap();
}

#[test]
fn remote_mode_wire_shape_is_stable_and_backward_compatible() {
    let without = new_session_request(None, &["sh"]);
    let without_json = serde_json::to_value(&without).unwrap();
    assert!(!without_json.as_object().unwrap().contains_key("remote"));

    let mut with = new_session_request_with_remote(None, &["sh"], Some(RemoteMode::Both));
    if let Request::NewSession { role, .. } = &mut with {
        *role = Some("worker".into());
    }
    let encoded = serde_json::to_string(&with).unwrap();
    assert_eq!(
        serde_json::to_value(&with).unwrap()["remote"],
        serde_json::json!("both")
    );
    assert!(
        encoded.find("\"role\":\"worker\"").unwrap() < encoded.find("\"remote\":\"both\"").unwrap()
    );
    let mut old = serde_json::to_value(&with).unwrap();
    old.as_object_mut().unwrap().remove("remote");
    let Request::NewSession { remote, .. } = serde_json::from_value(old).unwrap() else {
        panic!("expected new_session request");
    };
    assert_eq!(remote, None);
}
