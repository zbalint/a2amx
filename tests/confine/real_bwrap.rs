use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_bwrap_confines_the_session_process() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let state = root.path().join("state");
    let cwd = root.path().join("workspace");
    // Keep this outside the workspace and sandbox-private /tmp so it stays writable with a read-only $HOME.
    let external = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR"))?;
    let outside = external.path().join("outside.txt");
    let chmod_target = external.path().join("chmod-target");
    let hidden = external.path().join("hidden");
    let hidden_write = hidden.join("created");
    let result = cwd.join("result.txt");
    let inside = cwd.join("inside.txt");
    let fd_list = cwd.join("fds.txt");
    let no_new_privs = cwd.join("no-new-privs.txt");
    let done = cwd.join("done");
    let layout = a2amx::layout::Layout::new(state.clone());
    layout.ensure()?;
    let admin = layout.admin_token();
    std::fs::create_dir_all(&cwd)?;
    std::fs::create_dir_all(&hidden)?;
    std::fs::write(hidden.join("secret"), "hidden")?;
    std::fs::write(&outside, "unconfined")?;
    std::fs::write(&chmod_target, "chmod")?;
    std::fs::set_permissions(&chmod_target, std::fs::Permissions::from_mode(0o644))?;
    assert_eq!(std::fs::read_to_string(&outside)?, "unconfined");
    std::fs::remove_file(&outside)?;
    assert_eq!(std::fs::read_to_string(hidden.join("secret"))?, "hidden");
    std::fs::write(&admin, "admin")?;
    let bwrap_dir = bwrap_path_dir(Path::new("/usr/bin/bwrap"));
    let daemon = match start_confine_daemon(&state, bwrap_dir.path()).await {
        Ok(daemon) => daemon,
        Err(_error) if !require_bwrap() => return Ok(()),
        Err(error) => return Err(error),
    };
    let mut client = Client::connect(&state).await.context("connect client")?;
    let script = format!(
        "if printf outside > {outside}; then printf outside-ok > {result}; else printf outside-denied > {result}; fi; if printf hidden > {hidden_write}; then printf hidden-write-ok >> {result}; else printf hidden-write-denied >> {result}; fi; if chmod 600 {chmod_target}; then printf chmod-visible >> {result}; else printf chmod-denied >> {result}; fi; printf inside > {inside}; if test -e {hidden}; then printf visible >> {result}; else printf hidden >> {result}; fi; if test -r {admin}; then printf admin-visible >> {result}; else printf admin-hidden >> {result}; fi; if test -e /proc/{host_pid}; then printf proc-visible >> {result}; else printf proc-hidden >> {result}; fi; printf '%s' \"$(for fd in /proc/self/fd/*; do basename \"$fd\"; done)\" > {fd_list}; awk '/NoNewPrivs/{{print $2}}' /proc/self/status > {no_new_privs}; touch {done}; sleep 2",
        outside = outside.display(),
        chmod_target = chmod_target.display(),
        result = result.display(),
        inside = inside.display(),
        hidden = hidden.display(),
        hidden_write = hidden_write.display(),
        admin = admin.display(),
        fd_list = fd_list.display(),
        no_new_privs = no_new_privs.display(),
        done = done.display(),
        host_pid = std::process::id(),
    );
    let response = client
        .request(confined_shell_request(
            &cwd,
            &script,
            vec![],
            ConfineRequest {
                hide: vec![hidden.to_string_lossy().into_owned()],
                ..ConfineRequest::default()
            },
        ))
        .await
        .context("create confined session")?;
    let Response::Created { .. } = response else {
        if require_bwrap() {
            panic!("confined session creation failed: {response:?}");
        }
        daemon.shutdown().await?;
        return Ok(());
    };
    for _ in 0..100 {
        if done.exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let result_text = std::fs::read_to_string(&result).context("reading result marker")?;
    assert!(
        result_text.contains("outside-denied"),
        "result={result_text:?}"
    );
    assert!(!outside.exists(), "confined write escaped the workspace");
    assert!(result_text.contains("hidden-write-denied"));
    assert!(!hidden_write.exists(), "hidden tmpfs accepted a write");
    assert!(result_text.contains("chmod-denied"));
    assert_eq!(
        std::fs::metadata(&chmod_target)?.permissions().mode() & 0o777,
        0o644
    );
    assert!(inside.exists());
    assert!(result_text.contains("hidden"));
    assert!(result_text.contains("admin-hidden"));
    assert!(result_text.contains("proc-hidden"));
    let fd_values = std::fs::read_to_string(&fd_list)?
        .split_whitespace()
        .filter_map(|value| value.parse::<u32>().ok())
        .collect::<Vec<_>>();
    assert!(
        fd_values.iter().all(|fd| *fd <= 3),
        "unexpected descriptors: {fd_values:?}"
    );
    assert_eq!(std::fs::read_to_string(&no_new_privs)?.trim(), "1");
    daemon.shutdown().await?;
    Ok(())
}
#[test]
fn confined_session_survives_blocking_pool_thread_retirement() -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_keep_alive(Duration::from_millis(100))
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let root = tempfile::tempdir()?;
        let state = root.path().join("state");
        let cwd = root.path().join("workspace");
        std::fs::create_dir_all(&cwd)?;
        let bwrap_dir = bwrap_path_dir(Path::new("/usr/bin/bwrap"));
        let daemon = match start_confine_daemon(&state, bwrap_dir.path()).await {
            Ok(daemon) => daemon,
            Err(_error) if !require_bwrap() => return Ok(()),
            Err(error) => return Err(error),
        };
        let mut client = Client::connect(&state).await.context("connect client")?;
        let response = client
            .request(confined_shell_request(
                &cwd,
                "sleep 300",
                vec![],
                ConfineRequest::default(),
            ))
            .await
            .context("create confined session")?;
        let session_id = match response {
            Response::Created { session } => session,
            response => {
                if !require_bwrap() {
                    daemon.shutdown().await?;
                    return Ok(());
                }
                anyhow::bail!("confined session creation failed: {response:?}");
            }
        };
        for _ in 0..4 {
            tokio::task::spawn_blocking(|| {}).await?;
        }
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let sessions = match client.request(Request::List).await? {
            Response::Sessions { sessions } => sessions,
            response => anyhow::bail!("unexpected list response: {response:?}"),
        };
        let session = sessions
            .iter()
            .find(|session| session.id == session_id)
            .context("confined session missing from list")?;
        assert!(
            session.exit_code.is_none(),
            "confined session exited after blocking-pool retirement: {session:?}"
        );
        daemon.shutdown().await?;
        Ok(())
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn late_created_workspace_socket_is_visible_inside_confined_session() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let state = root.path().join("state");
    let cwd = root.path().join("workspace");
    std::fs::create_dir_all(&cwd)?;
    let socket_path = cwd.join("late.sock");
    let late_file = cwd.join("late-file");
    let result = cwd.join("socket-result");
    let bwrap_dir = bwrap_path_dir(Path::new("/usr/bin/bwrap"));
    let daemon = match start_confine_daemon(&state, bwrap_dir.path()).await {
        Ok(daemon) => daemon,
        Err(_error) if !require_bwrap() => return Ok(()),
        Err(error) => return Err(error),
    };
    let mut client = Client::connect(&state).await.context("connect client")?;
    let script = "until python3 -c 'import os,socket; s=socket.socket(socket.AF_UNIX); s.connect(os.environ[\"SOCKET\"]); s.send(b\"x\"); open(os.environ[\"RESULT\"], \"wb\").write(open(os.environ[\"FILE\"], \"rb\").read()+s.recv(1))'; do sleep 0.02; done";
    let response = client
        .request(confined_shell_request(
            &cwd,
            script,
            vec![
                ("SOCKET".into(), socket_path.to_string_lossy().into_owned()),
                ("RESULT".into(), result.to_string_lossy().into_owned()),
                ("FILE".into(), late_file.to_string_lossy().into_owned()),
            ],
            ConfineRequest::default(),
        ))
        .await?;
    let session_id = match response {
        Response::Created { session } => session,
        response => {
            if require_bwrap() {
                panic!("confined session creation failed: {response:?}");
            }
            daemon.shutdown().await?;
            return Ok(());
        }
    };
    std::fs::write(&late_file, "file")?;
    let listener = tokio::net::UnixListener::bind(&socket_path)?;
    let accepted = tokio::select! {
        accepted = timeout(Duration::from_secs(10), listener.accept()) => match accepted {
            Ok(result) => Some(result?),
            Err(_) => None,
        },
        exited = async {
            loop {
                let sessions = match client.request(Request::List).await? {
                    Response::Sessions { sessions } => sessions,
                    response => anyhow::bail!("unexpected list response: {response:?}"),
                };
                if let Some(summary) = sessions
                    .into_iter()
                    .find(|summary| summary.id == session_id && summary.exit_code.is_some())
                {
                    break Ok::<_, anyhow::Error>(summary);
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        } => {
            let summary = exited?;
            panic!(
                "confined session {session_id} exited before accepting socket: exit_code={:?}",
                summary.exit_code
            );
        }
    };
    let (mut stream, _) = match accepted {
        Some(accepted) => accepted,
        None => {
            let summary = match client.request(Request::List).await? {
                Response::Sessions { sessions } => sessions
                    .into_iter()
                    .find(|summary| summary.id == session_id),
                response => anyhow::bail!("unexpected list response: {response:?}"),
            };
            let exit_code = summary.as_ref().and_then(|summary| summary.exit_code);
            let hold_reason = summary
                .as_ref()
                .and_then(|summary| summary.hold_reason.as_deref());
            panic!(
                "confined session {session_id} did not accept socket: exit_code={exit_code:?}, hold_reason={hold_reason:?}"
            );
        }
    };
    let mut received = [0; 1];
    stream.read_exact(&mut received).await?;
    assert_eq!(received, [b'x']);
    stream.write_all(b"y").await?;
    for _ in 0..100 {
        if result.metadata().is_ok_and(|metadata| metadata.len() > 0) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(std::fs::read(&result)?, b"filey");
    daemon.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn confined_request_without_cwd_is_refused() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let state = root.path().join("state");
    std::fs::create_dir_all(&state)?;
    let bwrap_dir = bwrap_path_dir(Path::new("/usr/bin/bwrap"));
    let daemon = match start_confine_daemon(&state, bwrap_dir.path()).await {
        Ok(daemon) => daemon,
        Err(_error) if !require_bwrap() => return Ok(()),
        Err(error) => return Err(error),
    };
    let mut client = Client::connect(&state).await.context("connect client")?;
    let request = Request::Confine {
        confine: ConfineRequest::default(),
        request: Box::new(Request::NewSession {
            argv: vec!["true".into()],
            cols: 80,
            rows: 24,
            cwd: None,
            env: vec![],
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            activity: None,
            heartbeat: None,
            name: None,
            harness: Harness::Generic,
            deliver: None,
            team: None,
            role: None,
            remote: None,
        }),
    };
    let response = client.request(request).await?;
    assert!(matches!(
        response,
        Response::Error { message } if message.contains("explicit cwd")
    ));
    daemon.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_bwrap_probe_refuses_confined_session() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let state = root.path().join("state");
    let cwd = root.path().join("workspace");
    std::fs::create_dir_all(&cwd)?;
    let stub_dir = stub_path_dir(42);
    let daemon = start_confine_daemon(&state, stub_dir.path()).await?;
    let mut client = Client::connect(&state).await?;
    let marker = cwd.join("marker");
    let response = client
        .request(confined_shell_request(
            &cwd,
            &format!("printf ran > {}", marker.display()),
            vec![],
            ConfineRequest::default(),
        ))
        .await?;
    let Response::Error { message } = response else {
        panic!("failed probe unexpectedly created a session: {response:?}");
    };
    assert!(message.contains("bubblewrap") || message.contains("exit"));
    assert!(!marker.exists());
    daemon.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bwrap_probe_that_fails_after_version_refuses_confined_session() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let state = root.path().join("state");
    let cwd = root.path().join("workspace");
    std::fs::create_dir_all(&cwd)?;
    let stub_dir = probe_fail_stub_path_dir();
    let daemon = start_confine_daemon(&state, stub_dir.path()).await?;
    let mut client = Client::connect(&state).await?;
    let response = client
        .request(confined_shell_request(
            &cwd,
            "printf ran > marker",
            vec![],
            ConfineRequest::default(),
        ))
        .await?;
    assert!(matches!(response, Response::Error { .. }));
    assert!(!cwd.join("marker").exists());
    daemon.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_bwrap_path_refuses_confined_session() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let state = root.path().join("state");
    let cwd = root.path().join("workspace");
    std::fs::create_dir_all(&cwd)?;
    let empty_dir = tempfile::tempdir()?;
    let daemon = start_confine_daemon(&state, empty_dir.path()).await?;
    let mut client = Client::connect(&state).await?;
    let response = client
        .request(confined_shell_request(
            &cwd,
            "printf ran > marker",
            vec![],
            ConfineRequest::default(),
        ))
        .await?;
    assert!(matches!(response, Response::Error { .. }));
    assert!(!cwd.join("marker").exists());
    daemon.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn confined_generic_session_is_refused_when_env_lacks_the_signal_flags() -> anyhow::Result<()>
{
    let root = tempfile::tempdir()?;
    let state = root.path().join("state");
    let cwd = root.path().join("workspace");
    std::fs::create_dir_all(&cwd)?;
    let bwrap_dir = bwrap_probe_fail_stub_path_dir();
    let env_dir = stub_path_dir(42);
    let daemon =
        start_confine_daemon_with_env(&state, bwrap_dir.path(), Some(env_dir.path().join("bwrap")))
            .await?;
    let mut client = Client::connect(&state).await?;
    let response = client
        .request(confined_shell_request(
            &cwd,
            "printf ran > marker",
            vec![],
            ConfineRequest::default(),
        ))
        .await?;
    let Response::Error { message } = response else {
        panic!("expected confinement preflight error");
    };
    assert!(message.contains("--ignore-signal"));
    assert!(message.contains("--default-signal"));
    assert!(!cwd.join("marker").exists());
    daemon.shutdown().await?;
    Ok(())
}

async fn run_ctrl_c_case(
    state: &Path,
    cwd: &Path,
    script: String,
    ready: &Path,
    marker: Option<&Path>,
    expected_exit: i32,
) -> anyhow::Result<bool> {
    let bwrap_dir = bwrap_path_dir(Path::new("/usr/bin/bwrap"));
    let daemon = match start_confine_daemon(state, bwrap_dir.path()).await {
        Ok(daemon) => daemon,
        Err(_error) if !require_bwrap() => return Ok(false),
        Err(error) => return Err(error),
    };
    let mut client = Client::connect(state).await.context("connect client")?;
    let response = client
        .request(confined_shell_request(
            cwd,
            &script,
            vec![],
            ConfineRequest::default(),
        ))
        .await
        .context("create confined session")?;
    let session_id = match response {
        Response::Created { session } => session,
        response => {
            daemon.shutdown().await?;
            if !require_bwrap() {
                return Ok(false);
            }
            anyhow::bail!("confined session creation failed: {response:?}");
        }
    };

    common::eventually(|| async { ready.exists().then_some(()) }).await;
    let mut attachment = Client::connect(state)
        .await
        .context("connect attach client")?
        .attach(&session_id, false, 80, 24)
        .await
        .context("attach confined session")?;
    attachment
        .send(ClientFrame::Input(vec![0x03]))
        .await
        .context("send Ctrl-C input")?;
    drop(attachment);

    if let Some(marker) = marker {
        common::eventually(|| async { marker.exists().then_some(()) }).await;
    }
    let state_for_poll = state.to_owned();
    let session_id_for_poll = session_id.clone();
    let summary = common::eventually(|| {
        let state = state_for_poll.clone();
        let session_id = session_id_for_poll.clone();
        async move {
            let mut poll_client = Client::connect(&state).await.ok()?;
            let Response::Sessions { sessions } = poll_client.request(Request::List).await.ok()?
            else {
                return None;
            };
            sessions.into_iter().find(|session| {
                session.id == session_id && session.exit_code == Some(expected_exit)
            })
        }
    })
    .await;
    assert_eq!(summary.exit_code, Some(expected_exit));
    daemon.shutdown().await?;
    Ok(true)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ctrl_c_runs_the_trap_of_a_confined_shell() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let state = root.path().join("state");
    let cwd = root.path().join("workspace");
    std::fs::create_dir_all(&cwd)?;
    let ready = cwd.join("trap-ready");
    let marker = cwd.join("trap-marker");
    let script = format!(
        "trap 'echo got-int > {}; exit 7' INT; echo ready > {}; while :; do sleep 0.2; done",
        marker.display(),
        ready.display()
    );
    if !run_ctrl_c_case(&state, &cwd, script, &ready, Some(&marker), 7).await? {
        return Ok(());
    }
    assert_eq!(std::fs::read_to_string(marker)?, "got-int\n");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ctrl_c_does_not_end_a_confined_command_that_ignores_it() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let state = root.path().join("state");
    let cwd = root.path().join("workspace");
    std::fs::create_dir_all(&cwd)?;
    let ready = cwd.join("ignore-ready");
    let marker = cwd.join("ignore-marker");
    let script = format!(
        "trap '' INT; echo ready > {}; sleep 2; echo survived > {}",
        ready.display(),
        marker.display()
    );
    if !run_ctrl_c_case(&state, &cwd, script, &ready, Some(&marker), 0).await? {
        return Ok(());
    }
    assert_eq!(std::fs::read_to_string(marker)?, "survived\n");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ctrl_c_still_ends_a_confined_command_without_a_trap() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let state = root.path().join("state");
    let cwd = root.path().join("workspace");
    std::fs::create_dir_all(&cwd)?;
    let ready = cwd.join("sleep-ready");
    let script = format!("echo ready > {}; sleep 30", ready.display());
    if !run_ctrl_c_case(&state, &cwd, script, &ready, None, 130).await? {
        return Ok(());
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ctrl_c_reaches_a_confined_raw_mode_reader_as_input() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let state = root.path().join("state");
    let cwd = root.path().join("workspace");
    std::fs::create_dir_all(&cwd)?;
    let ready = cwd.join("raw-ready");
    let marker = cwd.join("raw-marker");
    let script = format!(
        "stty raw -echo; echo ready > {}; head -c1 > {}",
        ready.display(),
        marker.display()
    );
    if !run_ctrl_c_case(&state, &cwd, script, &ready, Some(&marker), 0).await? {
        return Ok(());
    }
    assert_eq!(std::fs::read(marker)?, [0x03]);
    Ok(())
}

#[test]
fn ssh_grant_runs_ssh_query_through_bwrap() -> anyhow::Result<()> {
    let bwrap = match confine::resolve_bwrap(None) {
        Ok(path) => path,
        Err(error) if !require_bwrap() => {
            eprintln!("skipping SSH bwrap probe: {error:#}");
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    match Command::new("ssh").arg("-V").output() {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("skipping SSH bwrap probe: ssh is not installed");
            return Ok(());
        }
        Err(error) => return Err(error).context("probe ssh availability"),
    }
    let fixture = Fixture::new();
    let plan = confine::plan(
        &ConfineRequest {
            grants: vec!["ssh".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
    )?
    .with_bwrap(bwrap);
    let wrapped = plan.wrap(vec!["ssh".into(), "-G".into(), "example.invalid".into()])?;
    match Command::new(&wrapped[0]).args(&wrapped[1..]).status() {
        Ok(status) => {
            assert_eq!(status.code(), Some(0));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("skipping SSH bwrap probe: ssh is not installed");
        }
        Err(error) => return Err(error).context("run SSH bwrap probe"),
    }
    Ok(())
}

#[test]
fn real_bubblewrap_cargo_grant_keeps_credentials_and_bin_protected() -> anyhow::Result<()> {
    let fixture = Fixture::new();
    let cargo = fixture.home.join(".cargo");
    let credentials = cargo.join("credentials.toml");
    let cargo_bin = cargo.join("bin");
    std::fs::create_dir_all(&cargo_bin)?;
    std::fs::write(&credentials, "credentials")?;
    std::fs::write(cargo_bin.join("installed"), "binary")?;
    let bwrap = match confine::resolve_bwrap(None) {
        Ok(path) => path,
        Err(error) if !require_bwrap() => {
            eprintln!("skipping cargo grant bwrap test: {error:#}");
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    let path_var = OsString::from(format!("{}:/usr/bin", cargo_bin.display()));
    let plan = confine::plan_with_path(
        &ConfineRequest {
            grants: vec!["cargo".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
        Some(path_var),
    )?
    .with_bwrap(bwrap);
    let new_file = cargo.join("new");
    let bin_file = cargo_bin.join("new");
    let script = format!(
        "set -eu; test ! -s '{}'; ! printf x > '{}'; ! printf x > '{}'; printf y > '{}'",
        credentials.display(),
        credentials.display(),
        bin_file.display(),
        new_file.display()
    );
    let wrapped = plan.wrap(vec!["sh".into(), "-c".into(), script])?;
    let status = Command::new(&wrapped[0])
        .args(&wrapped[1..])
        .status()
        .context("run cargo grant bwrap probe")?;
    assert_eq!(status.code(), Some(0));
    assert_eq!(std::fs::read_to_string(&credentials)?, "credentials");
    assert_eq!(std::fs::read_to_string(&new_file)?, "y");
    assert!(!bin_file.exists());
    Ok(())
}

#[test]
fn real_bubblewrap_hides_nested_credentials_but_allows_cache_writes() -> anyhow::Result<()> {
    let fixture = Fixture::new();
    let cargo = fixture.home.join(".cargo");
    std::fs::create_dir_all(&cargo)?;
    std::fs::write(cargo.join("credentials.toml"), "credentials")?;
    std::fs::write(fixture.cwd.join("Cargo.toml"), "")?;
    let bwrap = match confine::resolve_bwrap(None) {
        Ok(path) => path,
        Err(error) if !require_bwrap() => {
            eprintln!("skipping nested credential bwrap test: {error:#}");
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    let plan = confine::plan_with_path(
        &ConfineRequest {
            grants: vec!["cargo".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
        None,
    )?
    .with_bwrap(bwrap);
    let credentials = cargo.join("credentials.toml");
    let new_file = cargo.join("new");
    let script = format!(
        "set -eu; test ! -s '{}'; printf x > '{}'",
        credentials.display(),
        new_file.display()
    );
    let wrapped = plan.wrap(vec!["sh".into(), "-c".into(), script])?;
    let status = Command::new(&wrapped[0])
        .args(&wrapped[1..])
        .status()
        .context("run nested credential bwrap probe")?;
    assert_eq!(status.code(), Some(0));
    assert_eq!(std::fs::read_to_string(&credentials)?, "credentials");
    assert_eq!(std::fs::read_to_string(&new_file)?, "x");
    Ok(())
}

#[test]
fn real_bubblewrap_cache_grant_hides_the_token_and_writes_the_cache() -> anyhow::Result<()> {
    let fixture = Fixture::new();
    let cache = fixture.home.join(".cache");
    let token = cache.join("huggingface/token");
    std::fs::create_dir_all(token.parent().expect("token parent"))?;
    std::fs::write(&token, "token")?;
    let bwrap = match confine::resolve_bwrap(None) {
        Ok(path) => path,
        Err(error) if !require_bwrap() => {
            eprintln!("skipping cache grant bwrap test: {error:#}");
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    let plan = confine::plan_with_path(
        &ConfineRequest {
            grants: vec!["cache".into()],
            ..ConfineRequest::default()
        },
        Harness::Generic,
        &fixture.cwd,
        &fixture.host(),
        None,
    )?
    .with_bwrap(bwrap);
    let new_file = cache.join("new");
    let script = format!(
        "set -eu; test ! -s '{}'; printf x > '{}'",
        token.display(),
        new_file.display()
    );
    let wrapped = plan.wrap(vec!["sh".into(), "-c".into(), script])?;
    let status = Command::new(&wrapped[0])
        .args(&wrapped[1..])
        .status()
        .context("run cache grant bwrap probe")?;
    assert_eq!(status.code(), Some(0));
    assert_eq!(std::fs::read_to_string(&token)?, "token");
    assert_eq!(std::fs::read_to_string(&new_file)?, "x");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_bwrap_confined_tree_hides_state_run_and_secrets() -> anyhow::Result<()> {
    let base = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR"))?;
    let root = tempfile::tempdir_in(base.path())?;
    let cwd_dir = tempfile::tempdir_in(base.path())?;
    let cwd = cwd_dir.path().to_path_buf();
    let layout = a2amx::layout::Layout::new(root.path().to_path_buf());
    layout.ensure()?;
    let bin = root.path().join("bin/x");
    let team = layout.team_file();
    let discord = layout.discord_token();
    let db = layout.messages_db();
    let admin = layout.admin_token();
    let log = layout.daemon_log();
    std::fs::create_dir_all(bin.parent().unwrap())?;
    std::fs::write(&bin, "#!/bin/sh\n")?;
    std::fs::write(&team, "team = \"demo\"\n")?;
    std::fs::write(&discord, "discord-secret\n")?;
    std::fs::write(&admin, "admin-secret\n")?;
    std::fs::write(&log, "daemon log\n")?;
    let bwrap_dir = bwrap_path_dir(Path::new("/usr/bin/bwrap"));
    let bwrap_path = bwrap_dir.path().join("bwrap");
    if confine::preflight(&bwrap_path).is_err() {
        if require_bwrap() {
            anyhow::bail!("bubblewrap preflight failed");
        }
        return Ok(());
    }
    let daemon = start_confine_daemon(root.path(), bwrap_dir.path()).await?;
    assert!(db.exists(), "daemon must create the SQLite database");
    let mut client = Client::connect(root.path()).await?;
    let result = cwd.join("result");
    let done = cwd.join("done");
    let script = format!(
        "set -eu; cat {bin} >/dev/null; if printf x > {bin}; then printf bin-write-ok > {result}; else printf bin-write-denied > {result}; fi; cat {team} >/dev/null; if printf x > {team}; then printf team-write-ok >> {result}; else printf team-write-denied >> {result}; fi; cat {log} >/dev/null; if printf x > {log}; then printf log-write-ok >> {result}; else printf log-write-denied >> {result}; fi; if cat {admin} >/dev/null 2>&1; then printf admin-visible >> {result}; else printf admin-hidden >> {result}; fi; if cat {discord} >/dev/null 2>&1; then printf discord-visible >> {result}; else printf discord-hidden >> {result}; fi; if cat {db} >/dev/null 2>&1; then printf db-visible >> {result}; else printf db-hidden >> {result}; fi; if test -e {db}; then printf state-visible >> {result}; else printf state-hidden >> {result}; fi; if test -e {admin}; then printf run-visible >> {result}; else printf run-hidden >> {result}; fi; if test -e {discord}; then printf secrets-visible >> {result}; else printf secrets-hidden >> {result}; fi; touch {done}",
        bin = bin.display(),
        team = team.display(),
        log = log.display(),
        admin = admin.display(),
        discord = discord.display(),
        db = db.display(),
        result = result.display(),
        done = done.display(),
    );
    let response = client
        .request(confined_shell_request(
            &cwd,
            &script,
            vec![],
            ConfineRequest::default(),
        ))
        .await?;
    let Response::Created { .. } = response else {
        daemon.shutdown().await?;
        anyhow::bail!("confined session creation failed: {response:?}");
    };
    for _ in 0..100 {
        if done.exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let output = std::fs::read_to_string(result)?;
    assert!(output.contains("bin-write-denied"));
    assert!(output.contains("team-write-denied"));
    assert!(output.contains("log-write-denied"));
    assert!(output.contains("admin-hidden"));
    assert!(output.contains("discord-hidden"));
    assert!(output.contains("state-hidden"));
    assert!(output.contains("run-hidden"));
    assert!(output.contains("secrets-hidden"));
    daemon.shutdown().await?;
    Ok(())
}
