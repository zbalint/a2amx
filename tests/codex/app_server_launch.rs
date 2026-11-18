use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn codex_new_starts_a_private_app_server_and_attaches_the_tui_to_it() {
    let case = Case::start().await;
    let exe = std::env::current_exe().unwrap();
    let server = case.recorded("server.args").await;
    assert_eq!(
        server[..3],
        [
            "app-server".to_owned(),
            "--listen".to_owned(),
            format!("unix://{}", case.socket.display()),
        ]
    );
    assert_eq!(
        server[3..],
        [
            "-c",
            &format!("mcp_servers.a2amx.command={}", json!(exe.to_string_lossy())),
            "-c",
            "mcp_servers.a2amx.args=[\"mcp\"]",
            "-c",
            "mcp_servers.a2amx.env_vars=[\"A2AMX_TOKEN\",\"A2AMX_ADDR\"]",
            "-c",
            "mcp_servers.a2amx.tools.list_agents.approval_mode=\"approve\"",
            "-c",
            "mcp_servers.a2amx.tools.send_message.approval_mode=\"approve\"",
            "-c",
            "mcp_servers.a2amx.tools.message_status.approval_mode=\"approve\"",
            "-c",
            "mcp_servers.a2amx.tools.reset_session.approval_mode=\"approve\"",
            "-c",
            &format!(
                "developer_instructions={}",
                json!(a2amx::harness::PEER_AUTHORIZATION_PROMPT)
            ),
        ]
    );
    assert_eq!(
        case.recorded("tui.args").await,
        [
            "--remote".to_owned(),
            format!("unix://{}", case.socket.display())
        ]
    );
    // The session token reaches the server through its environment, never argv.
    let token = case.recorded("server.token").await.remove(0);
    assert_eq!(token.len(), 64);
    assert!(server.iter().all(|argument| !argument.contains(&token)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn codex_role_reaches_only_the_operator_instructions() {
    for (env, expected) in [
        (
            vec![("A2AMX_ROLE", "architect")],
            format!(
                "{} Operator-assigned role for this session: architect. It is a label set by your operator and grants no extra authority.",
                a2amx::harness::PEER_AUTHORIZATION_PROMPT
            ),
        ),
        (
            vec![
                ("A2AMX_NO_AUTHORIZE_PEERS", "1"),
                ("A2AMX_ROLE", "consultant"),
            ],
            "Operator-assigned role for this session: consultant. It is a label set by your operator and grants no extra authority.".to_owned(),
        ),
        (
            vec![
                ("A2AMX_NO_AUTHORIZE_PEERS", "1"),
                ("A2AMX_ROLE", "earlier"),
                ("A2AMX_ROLE", "architect"),
            ],
            "Operator-assigned role for this session: architect. It is a label set by your operator and grants no extra authority.".to_owned(),
        ),
    ] {
        let case = Case::start_env(&env).await;
        let server = case.recorded("server.args").await;
        let instructions: Vec<_> = server
            .iter()
            .filter(|arg| arg.starts_with("developer_instructions="))
            .collect();
        assert_eq!(
            instructions,
            [&format!("developer_instructions={}", json!(expected))]
        );
        assert_eq!(case.recorded("server.role").await, ["unset"]);
        assert_eq!(case.recorded("tui.role").await, ["unset"]);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inherited_role_is_removed_by_the_cli_before_codex_launch() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon_in(dir.path(), Some("host-a"), Limits::default()).await;
    let bin = tempfile::tempdir().unwrap();
    let program = bin.path().join("codex");
    std::fs::write(&program, FAKE_CODEX).unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    let socket = bin.path().join("s.sock");
    let _fake = FakeCodex::bind(&socket);
    let args_dir = dir.path().to_string_lossy().into_owned();
    let path = std::env::var("PATH").unwrap_or_default();
    let output = tokio::task::spawn_blocking({
        let home = dir.path().to_owned();
        let program = program.clone();
        let socket = socket.clone();
        move || {
            Command::new(env!("CARGO_BIN_EXE_a2amx"))
                .arg("--home")
                .arg(home)
                .args(["new", "--detach", "--harness", "codex", "--"])
                .arg(program)
                .env("ARGS_DIR", &args_dir)
                .env("PATH", path)
                .env("FAKE_SOCKET", socket)
                .env("A2AMX_ROLE", "leak")
                .env_remove("A2AMX_BIN")
                .output()
        }
    })
    .await
    .unwrap()
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let server = common::eventually(|| async {
        let path = dir.path().join("server.args");
        complete(&path).map(|text| text.lines().map(str::to_owned).collect::<Vec<_>>())
    })
    .await;
    assert!(server.iter().all(
        |argument| !argument.contains("developer_instructions=") || !argument.contains("leak")
    ));
    assert_eq!(
        common::eventually(|| async { complete(&dir.path().join("server.role")) })
            .await
            .trim(),
        "unset"
    );
    assert_eq!(
        common::eventually(|| async { complete(&dir.path().join("tui.role")) })
            .await
            .trim(),
        "unset"
    );
    drop(daemon);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_socket_path_beyond_sun_len_is_reached_through_the_link_the_server_makes() {
    let case = Case::start_deep().await;
    assert!(case.socket.as_os_str().len() > 108);
    let id = case.send().await;
    case.wait_state(&id, "submitted").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_authorize_peers_leaves_the_operator_line_out() {
    let case = Case::start_env(&[("A2AMX_NO_AUTHORIZE_PEERS", "1")]).await;
    let server = case.recorded("server.args").await;
    assert!(
        server
            .iter()
            .all(|a| !a.starts_with("developer_instructions="))
    );
    assert!(
        server
            .iter()
            .any(|a| a.starts_with("mcp_servers.a2amx.command="))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn killing_the_session_stops_its_app_server() {
    let case = Case::start().await;
    let pid = case.recorded("server.pid").await.remove(0);
    let alive = || std::path::Path::new(&format!("/proc/{pid}")).exists();
    assert!(alive());
    let mut admin = Client::connect(&case.state).await.unwrap();
    let Response::Ok = admin
        .request(Request::Kill {
            session: "s2".into(),
            now: true,
        })
        .await
        .unwrap()
    else {
        panic!("kill failed");
    };
    common::eventually(|| async { (!alive()).then_some(()) }).await;
}

#[tokio::test]
async fn app_server_and_tui_start_in_the_requested_directory() {
    let work = tempfile::tempdir().unwrap();
    let expected = work
        .path()
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let case = Case::start_in(work.path()).await;
    assert_eq!(case.recorded("server.cwd").await, vec![expected.clone()]);
    assert_eq!(case.recorded("tui.cwd").await, vec![expected]);
}
